//! The [`AsrEngine`]: an offline recognizer driven by a VAD to simulate
//! streaming with correction. See the crate docs for the overall approach.

use std::{path::Path, sync::Arc};

use anyhow::Context;
use sherpa_onnx::{
    OfflinePunctuation, OfflinePunctuationConfig, OfflineRecognizer, OfflineRecognizerConfig,
    VadModelConfig, VoiceActivityDetector,
};

use crate::{
    AsrConfig, PartialPunct, SAMPLE_RATE, Transcript, Transcription, TranscriptionSegment,
    load_wav_16k_mono,
};

/// Load-once, shareable heavy models (recognizer + optional punctuation) plus
/// the resolved config. `Send + Sync`; share across streams via `Arc`.
pub struct AsrModels {
    recognizer: OfflineRecognizer,
    /// Optional punctuation restorer, applied only to `Final` text.
    punct: Option<OfflinePunctuation>,
    config: AsrConfig,
}

impl AsrModels {
    /// Build the recognizer (+ optional punctuation) from `config`. Fails if a
    /// model path is missing/unreadable or the native library rejects it.
    pub fn load(config: AsrConfig) -> anyhow::Result<Arc<Self>> {
        anyhow::ensure!(
            !config.sense_voice_model.as_os_str().is_empty(),
            "AsrConfig.sense_voice_model path is empty"
        );
        anyhow::ensure!(
            !config.tokens.as_os_str().is_empty(),
            "AsrConfig.tokens path is empty"
        );
        anyhow::ensure!(
            !config.silero_vad_model.as_os_str().is_empty(),
            "AsrConfig.silero_vad_model path is empty"
        );
        let recognizer = build_recognizer(&config).context(
            "failed to create SenseVoice offline recognizer (check model/tokens paths and native lib)",
        )?;
        let punct = build_punct(&config)?;
        Ok(Arc::new(Self {
            recognizer,
            punct,
            config,
        }))
    }

    /// The resolved config (VAD params, cadence, language).
    pub fn config(&self) -> &AsrConfig {
        &self.config
    }

    /// Transcribe complete 16 kHz mono PCM, decoding each VAD segment only at its end.
    /// This blocking operation creates its own VAD and leaves live engines unchanged.
    pub fn transcribe_audio(self: &Arc<Self>, samples: &[f32]) -> anyhow::Result<Transcription> {
        let mut engine = AsrEngine::new(self.clone())?;
        let mut events = Vec::new();
        // Bound the engine's input buffer even when the caller supplies a long recording.
        for block in samples.chunks(SAMPLE_RATE as usize) {
            events.extend(engine.accept_samples(block, false));
        }
        events.extend(engine.flush());
        let segments: Vec<_> = events
            .into_iter()
            .filter_map(|event| match event {
                Transcript::Final {
                    text,
                    start,
                    duration,
                    ..
                } => Some(TranscriptionSegment {
                    text,
                    start,
                    duration,
                }),
                Transcript::Partial { .. } => None,
            })
            .collect();
        let text = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Transcription { text, segments })
    }

    /// Transcribe a complete 16 kHz WAV, downmixing multiple channels to mono.
    /// Other sample rates and formats must be converted before calling this method.
    /// The WAV is loaded into memory; model weights are reused across calls.
    pub fn transcribe_file(
        self: &Arc<Self>,
        path: impl AsRef<Path>,
    ) -> anyhow::Result<Transcription> {
        self.transcribe_audio(&load_wav_16k_mono(path)?)
    }
}

/// Real-time speech-to-text engine over one audio stream. Feed it 16 kHz mono
/// `f32` PCM and drain [`Transcript`] events. Not `Sync`; drive from one task.
pub struct AsrEngine {
    /// Shared heavy models.
    models: Arc<AsrModels>,
    /// Per-stream VAD (stateful — one per engine even though it is `Sync`).
    vad: VoiceActivityDetector,
    /// Samples not yet aligned to a full VAD window.
    pending: Vec<f32>,
    /// Audio accumulated for the currently-open utterance (interim `Partial`s).
    speech_buf: Vec<f32>,
    /// Samples fed into `speech_buf` since the last emitted `Partial`.
    since_partial: usize,
    /// Total samples accepted (debug/back-pressure only).
    total_samples: u64,
    /// Id of the utterance currently open; advanced on every `Final`.
    segment: u64,
    /// Samples handed to the VAD so far; the absolute clock VAD segments use.
    vad_samples: u64,
    /// Absolute sample index of `speech_buf[0]`.
    buf_start: u64,
    /// Absolute sample index up to which audio is already committed as a
    /// `Final`; the VAD segment's final decode skips it.
    committed_until: u64,
    /// Sentence prefix seen in the previous partial decode; committing needs
    /// two consecutive decodes to agree on it.
    last_candidate: Option<String>,
    /// Whether a `Partial` was emitted for the open utterance.
    partial_sent: bool,
    /// The most recent audio, prepended when speech starts: the VAD reports
    /// speech only after `min_speech_duration`, so without it partials (and
    /// early commits) lose the utterance's first syllable.
    preroll: Vec<f32>,
}

/// Recognizer output with per-token start times (seconds into the input).
struct Decoded {
    text: String,
    tokens: Vec<String>,
    timestamps: Vec<f32>,
}

/// Margin kept before the next sentence's first token when cutting audio, so
/// the cut does not clip its onset (SenseVoice timestamps step in ~60 ms).
const CUT_MARGIN_SECS: f32 = 0.12;

/// A token starting within this of the context end still belongs to the
/// committed text (half of SenseVoice's ~60 ms timestamp step).
const CONTEXT_TOL_SECS: f32 = 0.02;

/// Room kept below `max_decode_secs` when deciding to force a commit.
const FORCE_SAFETY_SECS: f32 = 0.25;

/// Text decoded from part of a segment and its sample range in the decoded audio.
struct Piece {
    text: String,
    start: usize,
    end: usize,
}

/// A stable sentence prefix to commit: its text and the cut point (seconds
/// into the decoded audio) where the remaining audio starts.
#[derive(Debug, PartialEq)]
struct Commit {
    text: String,
    cut_secs: f32,
}

impl AsrEngine {
    /// Build a per-stream engine sharing `models`. Builds a fresh VAD from the
    /// models' config; fails only if the native VAD cannot be created.
    pub fn new(models: Arc<AsrModels>) -> anyhow::Result<Self> {
        let vad = build_vad(models.config())
            .context("failed to create Silero VAD (check silero_vad_model path and native lib)")?;
        Ok(Self {
            models,
            vad,
            pending: Vec::new(),
            speech_buf: Vec::new(),
            since_partial: 0,
            total_samples: 0,
            segment: 0,
            vad_samples: 0,
            buf_start: 0,
            committed_until: 0,
            last_candidate: None,
            partial_sent: false,
            preroll: Vec::new(),
        })
    }

    /// Convenience: load models and build a single engine in one call (demo /
    /// single-stream use).
    pub fn from_config(config: AsrConfig) -> anyhow::Result<Self> {
        Self::new(AsrModels::load(config)?)
    }

    /// Push mono 16 kHz `f32` samples and return any transcription events
    /// produced. May emit zero or more `Partial`s and `Final`s per call.
    pub fn accept(&mut self, samples: &[f32]) -> Vec<Transcript> {
        self.accept_samples(samples, true)
    }

    fn accept_samples(&mut self, samples: &[f32], partials: bool) -> Vec<Transcript> {
        let mut out = Vec::new();
        self.total_samples += samples.len() as u64;
        self.pending.extend_from_slice(samples);

        let win = self.models.config().vad_window_size.max(1) as usize;
        let partial_every = self.models.config().partial_interval_samples();

        while self.pending.len() >= win {
            // Silero expects fixed-size windows; hand it exactly one at a time.
            let window: Vec<f32> = self.pending.drain(..win).collect();
            let window_start = self.vad_samples;
            self.vad.accept_waveform(&window);
            self.vad_samples += window.len() as u64;

            // While speech is ongoing, grow the interim buffer and re-decode it
            // periodically so the caller sees a self-correcting live transcript.
            if partials && self.vad.detected() {
                if self.speech_buf.is_empty() {
                    // Committed audio inside the preroll stays as left context;
                    // decodes drop the tokens it produces.
                    self.buf_start = window_start - self.preroll.len() as u64;
                    self.speech_buf.extend_from_slice(&self.preroll);
                }
                self.speech_buf.extend_from_slice(&window);
                self.since_partial += window.len();
                if self.since_partial >= partial_every {
                    self.partial(&mut out);
                }
            }
            if partials {
                let keep = self.models.config().preroll_samples();
                self.preroll.extend_from_slice(&window);
                let excess = self.preroll.len().saturating_sub(keep);
                self.preroll.drain(..excess);
            }

            // Any segment the VAD has closed becomes a committed Final.
            self.drain_segments(&mut out);
        }

        out
    }

    /// Re-decode the open utterance, commit a stable leading sentence early when
    /// enabled, force a commit when the buffer nears `max_decode_secs`, and
    /// emit the rest as a `Partial`.
    fn partial(&mut self, out: &mut Vec<Transcript>) {
        // Reset even when nothing decodes, so silence-like audio is not
        // re-decoded on every VAD window.
        self.since_partial = 0;
        let Some(mut decoded) = self.decode_buf() else {
            if self.near_limit() {
                // No text to commit; drop the audio to stay within the limit.
                self.cut_buffer(self.speech_buf.len());
            }
            return;
        };

        if self.models.config().early_commit {
            let candidate = stable_prefix(&decoded.tokens, &decoded.timestamps);
            let agreed = candidate.as_ref().map(|c| &c.text) == self.last_candidate.as_ref();
            match candidate {
                Some(commit) if agreed => {
                    self.commit(commit, out);
                    // Show only what is left after the committed sentence.
                    match self.decode_buf() {
                        Some(rest) => decoded = rest,
                        None => return,
                    }
                }
                other => self.last_candidate = other.map(|c| c.text),
            }
        }

        // Each forced cut keeps at most half of the new audio, so a couple of
        // rounds always suffice; the bound only guards against a stuck cut.
        for _ in 0..4 {
            if !self.near_limit() {
                break;
            }
            let sr = SAMPLE_RATE as f32;
            let context = self.context_samples() as f32 / sr;
            let len = self.speech_buf.len() as f32 / sr;
            let min_cut = context + (len - context) / 2.0;
            let commit = forced_cut(&decoded.tokens, &decoded.timestamps, min_cut).unwrap_or(
                // A single token (or no timestamps): commit it all.
                Commit {
                    text: decoded.text.clone(),
                    cut_secs: len,
                },
            );
            log::debug!(
                "forced commit: {len:.2}s buffer, cut at {:.2}s",
                commit.cut_secs
            );
            self.commit(commit, out);
            match self.decode_buf() {
                Some(rest) => decoded = rest,
                None => return,
            }
        }

        let text = partial_text(
            decoded.text,
            self.models.punct.is_some(),
            self.models.config().partial_punct,
        );
        out.push(Transcript::Partial {
            segment: self.segment,
            text,
        });
        self.partial_sent = true;
    }

    /// Emit `commit` as a `Final`, drop its audio from the interim buffer
    /// (keeping `commit_overlap` as context), and open a new utterance id for
    /// what follows.
    fn commit(&mut self, commit: Commit, out: &mut Vec<Transcript>) {
        let sr = SAMPLE_RATE as f32;
        let cut = ((commit.cut_secs * sr) as usize).min(self.speech_buf.len());
        let start = self.buf_start.max(self.committed_until);
        self.cut_buffer(cut);
        out.push(Transcript::Final {
            segment: self.segment,
            text: self.finalize_text(commit.text),
            start: start as f32 / sr,
            duration: self.committed_until.saturating_sub(start) as f32 / sr,
        });
        log::debug!("early commit at {:.2}s", self.committed_until as f32 / sr);
        self.segment += 1;
        self.last_candidate = None;
        self.partial_sent = false;
    }

    /// Mark the first `cut` samples of the interim buffer as committed and drop
    /// them, except the last `commit_overlap` kept as left context.
    fn cut_buffer(&mut self, cut: usize) {
        self.committed_until = self.committed_until.max(self.buf_start + cut as u64);
        let drop = cut.saturating_sub(self.models.config().overlap_samples());
        self.speech_buf.drain(..drop);
        self.buf_start += drop as u64;
    }

    /// Samples at the head of the interim buffer that are already committed.
    fn context_samples(&self) -> usize {
        (self.committed_until.saturating_sub(self.buf_start) as usize).min(self.speech_buf.len())
    }

    /// Whether the next partial decode could exceed `max_decode_secs`.
    fn near_limit(&self) -> bool {
        let config = self.models.config();
        let Some(max) = config.max_decode_samples() else {
            return false;
        };
        let safety = (FORCE_SAFETY_SECS * SAMPLE_RATE as f32) as usize;
        self.speech_buf.len() + config.partial_interval_samples() + safety > max
    }

    /// Decode the interim buffer, dropping tokens from its committed context.
    fn decode_buf(&self) -> Option<Decoded> {
        let t_dec = std::time::Instant::now();
        let decoded = self.decode_after(&self.speech_buf, self.context_samples());
        log::debug!(
            "partial decode took {:?} for {:.2}s audio",
            t_dec.elapsed(),
            self.speech_buf.len() as f32 / SAMPLE_RATE as f32
        );
        decoded
    }

    /// Signal end of stream: flush the VAD's trailing speech and emit any
    /// remaining `Final`s. Resets the interim buffer.
    pub fn flush(&mut self) -> Vec<Transcript> {
        let mut out = Vec::new();

        // Feed leftover (sub-window) samples, then force the VAD to close any
        // in-progress speech into its output queue.
        if !self.pending.is_empty() {
            let tail = std::mem::take(&mut self.pending);
            self.vad.accept_waveform(&tail);
            self.vad_samples += tail.len() as u64;
        }
        self.vad.flush();
        self.drain_segments(&mut out);

        self.speech_buf.clear();
        self.since_partial = 0;
        out
    }

    /// Move every completed VAD segment into `out` as `Final`s, decoding the
    /// authoritative (silence-trimmed) segment samples.
    fn drain_segments(&mut self, out: &mut Vec<Transcript>) {
        while !self.vad.is_empty() {
            if let Some(segment) = self.vad.front() {
                let seg_start = segment.start().max(0) as u64;
                let samples = segment.samples();
                let seg_end = seg_start + samples.len() as u64;
                // Skip audio an early commit already finalized, re-decoding up
                // to `commit_overlap` of it as left context.
                let skip =
                    (self.committed_until.saturating_sub(seg_start) as usize).min(samples.len());
                let from = skip.saturating_sub(self.models.config().overlap_samples());
                let t_dec = std::time::Instant::now();
                let pieces = self.decode_bounded(&samples[from..], skip - from);
                log::debug!(
                    "final decode took {:?} for {:.2}s audio",
                    t_dec.elapsed(),
                    (samples.len() - from) as f32 / SAMPLE_RATE as f32
                );
                let sr = SAMPLE_RATE as f32;
                let at = |offset: usize| (seg_start + (from + offset) as u64) as f32 / sr;
                let emitted = !pieces.is_empty();
                for (i, piece) in pieces.into_iter().enumerate() {
                    // The first piece closes the open utterance; each further
                    // one gets its own id (the last is advanced below).
                    if i > 0 {
                        self.segment += 1;
                    }
                    out.push(Transcript::Final {
                        segment: self.segment,
                        text: self.finalize_text(piece.text),
                        start: at(piece.start),
                        duration: at(piece.end) - at(piece.start),
                    });
                }
                // Close a shown partial even when the tail decodes empty.
                if !emitted && self.partial_sent {
                    out.push(Transcript::Final {
                        segment: self.segment,
                        text: String::new(),
                        start: at(skip - from),
                        duration: at(samples.len() - from) - at(skip - from),
                    });
                }
                // Later speech must not re-decode this segment's audio.
                self.committed_until = self.committed_until.max(seg_end);
            }
            self.vad.pop();
            // The utterance ended; the next one gets a new id.
            self.segment += 1;
            self.speech_buf.clear();
            self.since_partial = 0;
            self.last_candidate = None;
            self.partial_sent = false;
        }
    }

    /// Finalize a segment's text: when a punctuation model is configured, strip
    /// any recognizer punctuation and restore it in one pass; otherwise return
    /// the recognizer text unchanged.
    fn finalize_text(&self, text: String) -> String {
        match &self.models.punct {
            Some(p) => {
                let clean = strip_punct(&text);
                let t = std::time::Instant::now();
                let out = p.add_punctuation(&clean).unwrap_or(clean);
                log::debug!(
                    "punct add_punctuation took {:?} ({} chars)",
                    t.elapsed(),
                    out.chars().count()
                );
                out
            }
            None => text,
        }
    }

    /// Decode `samples`, whose first `context` samples are committed audio,
    /// within `max_decode_secs`: longer audio is cut at a pause into several
    /// decodes, each re-reading `commit_overlap` before the previous cut.
    fn decode_bounded(&self, samples: &[f32], mut context: usize) -> Vec<Piece> {
        let config = self.models.config();
        let max = config.max_decode_samples().unwrap_or(usize::MAX);
        let overlap = config.overlap_samples();
        let sr = SAMPLE_RATE as f32;
        let mut pieces = Vec::new();
        let mut pos = 0;
        loop {
            let rest = &samples[pos..];
            if rest.len() <= max {
                if let Some(d) = self.decode_after(rest, context) {
                    pieces.push(Piece {
                        text: d.text,
                        start: pos + context,
                        end: samples.len(),
                    });
                }
                return pieces;
            }
            let window = &rest[..max];
            let decoded = self.decode_after(window, context);
            let context_secs = context as f32 / sr;
            let min_cut = context_secs + (max as f32 / sr - context_secs) / 2.0;
            let commit = decoded
                .as_ref()
                .and_then(|d| forced_cut(&d.tokens, &d.timestamps, min_cut));
            let (text, cut) = match (commit, decoded) {
                (Some(c), _) => (Some(c.text), (c.cut_secs * sr) as usize),
                (None, d) => (d.map(|d| d.text), window.len()),
            };
            // Always move past the context so the loop advances.
            let cut = cut.max(context + 1).min(window.len());
            if let Some(text) = text {
                pieces.push(Piece {
                    text,
                    start: pos + context,
                    end: pos + cut,
                });
            }
            context = overlap.min(cut);
            pos += cut - context;
        }
    }

    /// Decode `samples` whose first `context` samples were already committed,
    /// keeping only the tokens that start after them.
    fn decode_after(&self, samples: &[f32], context: usize) -> Option<Decoded> {
        let decoded = self.decode_full(samples)?;
        if context == 0 {
            return Some(decoded);
        }
        drop_context(decoded, context as f32 / SAMPLE_RATE as f32)
    }

    /// Run the offline recognizer over `samples`, returning the trimmed text
    /// with its tokens and their start times, if the text is non-empty.
    fn decode_full(&self, samples: &[f32]) -> Option<Decoded> {
        if samples.is_empty() {
            return None;
        }
        let stream = self.models.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE, samples);
        self.models.recognizer.decode(&stream);
        let result = stream.get_result()?;
        let text = result.text.trim();
        if text.is_empty() {
            return None;
        }
        Some(Decoded {
            text: text.to_string(),
            tokens: result.tokens,
            timestamps: result.timestamps.unwrap_or_default(),
        })
    }
}

fn build_recognizer(config: &AsrConfig) -> Option<OfflineRecognizer> {
    let mut c = OfflineRecognizerConfig::default();
    c.model_config.sense_voice.model =
        Some(config.sense_voice_model.to_string_lossy().into_owned());
    c.model_config.sense_voice.language = Some(config.language.clone());
    c.model_config.sense_voice.use_itn = config.use_itn;
    c.model_config.tokens = Some(config.tokens.to_string_lossy().into_owned());
    c.model_config.num_threads = config.num_threads;
    c.model_config.provider = Some(config.provider.clone());
    c.model_config.debug = config.debug;
    OfflineRecognizer::create(&c)
}

fn build_vad(config: &AsrConfig) -> Option<VoiceActivityDetector> {
    let mut c = VadModelConfig::default();
    c.silero_vad.model = Some(config.silero_vad_model.to_string_lossy().into_owned());
    c.silero_vad.threshold = config.vad_threshold;
    c.silero_vad.min_silence_duration = config.min_silence_duration;
    c.silero_vad.min_speech_duration = config.min_speech_duration;
    c.silero_vad.max_speech_duration = config.max_speech_duration;
    c.silero_vad.window_size = config.vad_window_size;
    c.sample_rate = SAMPLE_RATE;
    c.debug = config.debug;
    VoiceActivityDetector::create(&c, config.vad_buffer_seconds)
}

/// Build the optional CT-Transformer punctuation restorer. `Ok(None)` when no
/// `punct_model` is configured; `Err` when a configured model fails to load.
fn build_punct(config: &AsrConfig) -> anyhow::Result<Option<OfflinePunctuation>> {
    let Some(path) = &config.punct_model else {
        return Ok(None);
    };
    let mut c = OfflinePunctuationConfig::default();
    c.model.ct_transformer = Some(path.to_string_lossy().into_owned());
    c.model.num_threads = config.num_threads;
    c.model.debug = config.debug;
    let punct = OfflinePunctuation::create(&c)
        .context("failed to create punctuation model (check punct_model path and native lib)")?;
    Ok(Some(punct))
}

/// A mark after which a short sentence may be committed: sentence ends plus
/// `，`/`；`, which SenseVoice puts between sentences of continuous speech.
/// `、` (list separator) and `：` are excluded.
fn is_sentence_end(token: &str) -> bool {
    matches!(
        token.trim(),
        "。" | "！" | "？" | "!" | "?" | "." | "…" | "，" | "," | "；" | ";"
    )
}

/// Join recognizer tokens into text (SentencePiece `▁` marks a word start).
fn join_tokens(tokens: &[String]) -> String {
    tokens.concat().replace('\u{2581}', " ").trim().to_string()
}

/// The longest leading run of whole short sentences that is followed by more
/// speech, with the cut point before the next sentence's first token.
/// The final token is never a boundary: SenseVoice ends every decode with a
/// sentence mark, so it says nothing about where the speaker paused. `None`
/// without timestamps or without an inner sentence end.
fn stable_prefix(tokens: &[String], timestamps: &[f32]) -> Option<Commit> {
    if tokens.len() < 2 || timestamps.len() != tokens.len() {
        return None;
    }
    let end = (0..tokens.len() - 1)
        .rev()
        .find(|&i| is_sentence_end(&tokens[i]) && !is_sentence_end(&tokens[i + 1]))?;
    let text = join_tokens(&tokens[..=end]);
    if text.chars().all(|c| is_punct(c) || c.is_whitespace()) {
        return None;
    }
    let next = timestamps[end + 1];
    let cut_secs = (next - CUT_MARGIN_SECS).max(timestamps[end]).max(0.0);
    Some(Commit { text, cut_secs })
}

/// Where to cut audio that is too long for one decode, given its decode
/// (timestamps in seconds into that audio). Only boundaries whose next token
/// starts at or after `min_secs` count, so what remains stays short. Prefers
/// the last sentence mark, then the widest gap between token starts (a
/// pause), then the latest boundary at any position; never puts a mark at the
/// start of the remainder. `None` without two tokens, timestamps or such a
/// boundary.
fn forced_cut(tokens: &[String], timestamps: &[f32], min_secs: f32) -> Option<Commit> {
    if tokens.len() < 2 || timestamps.len() != tokens.len() {
        return None;
    }
    let is_mark = |t: &str| t.trim().chars().all(is_punct);
    let boundaries: Vec<usize> = (0..tokens.len() - 1)
        .filter(|&i| timestamps[i + 1] >= min_secs && !is_mark(&tokens[i + 1]))
        .collect();
    let gap = |i: usize| timestamps[i + 1] - timestamps[i];
    let end = boundaries
        .iter()
        .rev()
        .copied()
        .find(|&i| is_sentence_end(&tokens[i]))
        // `max_by` keeps the last of equal gaps: cut as late as possible.
        .or_else(|| {
            boundaries
                .iter()
                .copied()
                .max_by(|&a, &b| gap(a).total_cmp(&gap(b)))
        })
        .or_else(|| {
            (0..tokens.len() - 1)
                .rev()
                .find(|&i| !is_mark(&tokens[i + 1]))
        })?;
    let text = join_tokens(&tokens[..=end]);
    if text.chars().all(|c| is_punct(c) || c.is_whitespace()) {
        return None;
    }
    let cut_secs = (timestamps[end + 1] - CUT_MARGIN_SECS)
        .max(timestamps[end])
        .max(0.0);
    Some(Commit { text, cut_secs })
}

/// Keep only the tokens of `decoded` that start after `context_secs` (the
/// committed audio in front), rebuilding the text from them. `None` if
/// nothing but punctuation is left. Without timestamps it cannot tell the
/// context apart and returns `decoded` unchanged.
fn drop_context(decoded: Decoded, context_secs: f32) -> Option<Decoded> {
    if decoded.timestamps.len() != decoded.tokens.len() {
        return Some(decoded);
    }
    let first = decoded
        .timestamps
        .iter()
        .position(|&t| t > context_secs + CONTEXT_TOL_SECS)?;
    let tokens = decoded.tokens[first..].to_vec();
    let text = join_tokens(&tokens);
    if text.chars().all(|c| is_punct(c) || c.is_whitespace()) {
        return None;
    }
    Some(Decoded {
        text,
        tokens,
        timestamps: decoded.timestamps[first..].to_vec(),
    })
}

/// Interim text for a `Partial`. Punctuation is stripped only when a
/// punctuation model will re-punctuate the final and `mode` asks for it.
fn partial_text(text: String, has_punct_model: bool, mode: PartialPunct) -> String {
    if has_punct_model && mode == PartialPunct::Strip {
        strip_punct(&text)
    } else {
        text
    }
}

/// Drop sentence punctuation (ASCII + CJK) from `text`. Used to keep interim
/// `Partial`s punctuation-free and to normalize a segment before re-punctuating.
fn strip_punct(text: &str) -> String {
    text.chars().filter(|c| !is_punct(*c)).collect()
}

fn is_punct(c: char) -> bool {
    matches!(
        c,
        ',' | '.'
            | '!'
            | '?'
            | ';'
            | ':'
            | '，'
            | '。'
            | '！'
            | '？'
            | '、'
            | '；'
            | '：'
            | '…'
            | '—'
            | '～'
            | '·'
            | '「'
            | '」'
            | '『'
            | '』'
            | '（'
            | '）'
            | '《'
            | '》'
            | '【'
            | '】'
            | '〈'
            | '〉'
            | '\u{201C}'
            | '\u{201D}'
            | '\u{2018}'
            | '\u{2019}'
    )
}

#[cfg(test)]
#[path = "engine_test.rs"]
mod engine_test;
