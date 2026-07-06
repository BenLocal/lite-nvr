//! The [`AsrEngine`]: an offline recognizer driven by a VAD to simulate
//! streaming with correction. See the crate docs for the overall approach.

use std::sync::Arc;

use anyhow::Context;
use sherpa_onnx::{
    OfflinePunctuation, OfflinePunctuationConfig, OfflineRecognizer, OfflineRecognizerConfig,
    VadModelConfig, VoiceActivityDetector,
};

use crate::{AsrConfig, SAMPLE_RATE, Transcript};

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
        let mut out = Vec::new();
        self.total_samples += samples.len() as u64;
        self.pending.extend_from_slice(samples);

        let win = self.models.config().vad_window_size.max(1) as usize;
        let partial_every = self.models.config().partial_interval_samples();

        while self.pending.len() >= win {
            // Silero expects fixed-size windows; hand it exactly one at a time.
            let window: Vec<f32> = self.pending.drain(..win).collect();
            self.vad.accept_waveform(&window);

            // While speech is ongoing, grow the interim buffer and re-decode it
            // periodically so the caller sees a self-correcting live transcript.
            if self.vad.detected() {
                self.speech_buf.extend_from_slice(&window);
                self.since_partial += window.len();
                if self.since_partial >= partial_every {
                    let t_dec = std::time::Instant::now();
                    let decoded = self.decode(&self.speech_buf);
                    log::debug!(
                        "partial decode took {:?} for {:.2}s audio",
                        t_dec.elapsed(),
                        self.speech_buf.len() as f32 / SAMPLE_RATE as f32
                    );
                    if let Some(text) = decoded {
                        self.since_partial = 0;
                        // With a punctuation model, keep interim text
                        // punctuation-free so partials show only the
                        // recognizer's corrections.
                        let text = if self.models.punct.is_some() {
                            strip_punct(&text)
                        } else {
                            text
                        };
                        out.push(Transcript::Partial { text });
                    }
                }
            }

            // Any segment the VAD has closed becomes a committed Final.
            self.drain_segments(&mut out);
        }

        out
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
        }
        self.vad.flush();
        self.drain_segments(&mut out);

        self.speech_buf.clear();
        self.since_partial = 0;
        out
    }

    /// Move every completed VAD segment into `out` as a `Final`, decoding the
    /// authoritative (silence-trimmed) segment samples.
    fn drain_segments(&mut self, out: &mut Vec<Transcript>) {
        while !self.vad.is_empty() {
            if let Some(segment) = self.vad.front() {
                let samples = segment.samples();
                let t_dec = std::time::Instant::now();
                let decoded = self.decode(samples);
                log::debug!(
                    "final decode took {:?} for {:.2}s audio",
                    t_dec.elapsed(),
                    samples.len() as f32 / SAMPLE_RATE as f32
                );
                if let Some(text) = decoded {
                    let sr = SAMPLE_RATE as f32;
                    out.push(Transcript::Final {
                        text: self.finalize_text(text),
                        start: segment.start() as f32 / sr,
                        duration: segment.n() as f32 / sr,
                    });
                }
            }
            self.vad.pop();
            // The utterance ended; drop its interim buffer.
            self.speech_buf.clear();
            self.since_partial = 0;
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

    /// Run the offline recognizer over `samples`, returning the trimmed text if
    /// it is non-empty.
    fn decode(&self, samples: &[f32]) -> Option<String> {
        if samples.is_empty() {
            return None;
        }
        let stream = self.models.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE, samples);
        self.models.recognizer.decode(&stream);
        let text = stream.get_result()?.text;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
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
