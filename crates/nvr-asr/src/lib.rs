//! Speech-to-text for complete audio and simulated streaming with correction,
//! using an **offline** ASR model.
//!
//! A streaming (online) recognizer emits words as they arrive but is usually
//! less accurate. Instead, this crate pairs a [Silero VAD] with an **offline**
//! (non-streaming) [SenseVoice] recognizer: the VAD finds speech boundaries,
//! and while a segment is still open the recognizer re-decodes the accumulated
//! audio every so often, producing an interim [`Transcript::Partial`] that
//! *supersedes and may correct* the previous one. When the VAD closes the
//! segment, the finalized samples are decoded into a [`Transcript::Final`].
//!
//! The [`AsrEngine`] is source-agnostic: feed it 16 kHz mono `f32` PCM via
//! [`AsrEngine::accept`] and drain [`Transcript`] events; call
//! [`AsrEngine::flush`] at end of stream. See the `nvr-asr-demo` binary for a
//! WAV-file driver supporting both modes.
//!
//! For complete recordings, use [`AsrModels::transcribe_audio`] or
//! [`AsrModels::transcribe_file`]. These reuse the loaded models and decode
//! only finalized VAD segments, without the repeated interim recognition.
//!
//! [Silero VAD]: https://github.com/snakers4/silero-vad
//! [SenseVoice]: https://github.com/FunAudioLLM/SenseVoice

use std::{path::PathBuf, time::Duration};

// Links the libstdc++ shim for older cross toolchains.
use nvr_sherpa_compat as _;

mod engine;
mod wav;

pub use engine::{AsrEngine, AsrModels};
pub use wav::{WavAudio, load_wav_16k_mono};

/// The canonical sample rate SenseVoice + Silero VAD expect.
pub const SAMPLE_RATE: i32 = 16_000;

/// A finalized speech segment in a complete-audio transcription.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionSegment {
    pub text: String,
    /// Start time in seconds from the beginning of the input audio.
    pub start: f32,
    /// Speech duration in seconds.
    pub duration: f32,
}

/// Complete-audio result. Segments are chronological; `text` joins them with newlines.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcription {
    pub text: String,
    pub segments: Vec<TranscriptionSegment>,
}

/// How interim [`Transcript::Partial`] text is punctuated when a punctuation
/// model is configured. Finals are always re-punctuated by that model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PartialPunct {
    /// Strip punctuation so partials show only the recognizer's corrections.
    #[default]
    Strip,
    /// Keep the recognizer's own punctuation (SenseVoice with `use_itn`). Costs
    /// no extra inference, but trailing marks may change as audio arrives.
    Recognizer,
}

impl std::str::FromStr for PartialPunct {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "strip" | "none" => Ok(Self::Strip),
            "recognizer" => Ok(Self::Recognizer),
            other => anyhow::bail!(
                "unknown partial punctuation mode {other:?} (expected strip|recognizer)"
            ),
        }
    }
}

/// Configuration for an [`AsrEngine`]. Construct with [`AsrConfig::new`] (the
/// three required model paths) and adjust fields as needed; [`Default`] fills
/// in sensible values but leaves the paths empty.
#[derive(Debug, Clone)]
pub struct AsrConfig {
    // --- required model assets ---
    /// Path to the SenseVoice ONNX model (e.g. `model.int8.onnx`).
    pub sense_voice_model: PathBuf,
    /// Path to the recognizer's `tokens.txt`.
    pub tokens: PathBuf,
    /// Path to the Silero VAD ONNX model (`silero_vad.onnx`).
    pub silero_vad_model: PathBuf,

    /// Optional CT-Transformer punctuation model (`model.onnx` from a
    /// `sherpa-onnx-punct-*` release). When set, each [`Transcript::Final`] is
    /// punctuated by this model in one pass, and interim
    /// [`Transcript::Partial`]s follow [`AsrConfig::partial_punct`]. When
    /// `None`, punctuation follows the recognizer (`use_itn`).
    pub punct_model: Option<PathBuf>,
    /// Punctuation shown on interim [`Transcript::Partial`]s when
    /// `punct_model` is set. Without a punctuation model partials always keep
    /// the recognizer's text unchanged.
    pub partial_punct: PartialPunct,

    // --- recognizer options ---
    /// SenseVoice language: `auto`, `zh`, `en`, `ja`, `ko`, or `yue`.
    pub language: String,
    /// Enable inverse text normalization (digits/punctuation in the output).
    pub use_itn: bool,
    /// Recognizer threads.
    pub num_threads: i32,
    /// Recognizer execution provider: `cpu`, or `rknn` for the Rockchip NPU
    /// (needs an RKNN `.rknn` SenseVoice model and the RKNN build of
    /// sherpa-onnx, see the `sherpa-shared` feature). RKNN models take a fixed
    /// input length; set [`AsrConfig::max_decode_secs`] to it.
    pub provider: String,
    /// Longest audio, in seconds, passed to one recognizer decode; `None` for
    /// no limit. RKNN models are converted for a fixed length (5 s, 10 s, ...)
    /// and drop audio beyond it, and the VAD's `max_speech_duration` is only a
    /// hint, so with a limit the engine commits text early at a pause and
    /// splits long segments to stay within it.
    pub max_decode_secs: Option<f32>,
    /// Seconds of already-committed audio decoded again in front of the
    /// remaining audio after a commit, so the words right after the cut are
    /// recognized with left context. Tokens starting inside it are dropped.
    pub commit_overlap: f32,
    /// Print sherpa-onnx internal debug logs.
    pub debug: bool,

    // --- VAD tuning ---
    /// Speech probability threshold (0..1).
    pub vad_threshold: f32,
    /// Silence (seconds) that ends a segment.
    pub min_silence_duration: f32,
    /// Minimum speech (seconds) for a segment to count.
    pub min_speech_duration: f32,
    /// Force-split a segment after this many seconds of continuous speech.
    pub max_speech_duration: f32,
    /// Silero window size in samples (512 for 16 kHz).
    pub vad_window_size: i32,
    /// VAD internal ring-buffer length, in seconds.
    pub vad_buffer_seconds: f32,

    // --- streaming-correction cadence ---
    /// How often to re-decode an in-progress segment for a fresh
    /// [`Transcript::Partial`]. Smaller = snappier partials, more CPU.
    pub partial_interval: Duration,
    /// Commit a leading sentence as a [`Transcript::Final`] while the speaker
    /// keeps talking, once two consecutive partial decodes agree on it, and
    /// drop its audio so later re-decodes cover only the rest. Needs the
    /// recognizer's token timestamps (SenseVoice provides them).
    pub early_commit: bool,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            sense_voice_model: PathBuf::new(),
            tokens: PathBuf::new(),
            silero_vad_model: PathBuf::new(),
            punct_model: None,
            partial_punct: PartialPunct::default(),
            language: "auto".to_string(),
            use_itn: true,
            num_threads: 2,
            provider: "cpu".to_string(),
            max_decode_secs: None,
            commit_overlap: 0.5,
            debug: false,
            vad_threshold: 0.5,
            min_silence_duration: 0.25,
            min_speech_duration: 0.25,
            max_speech_duration: 10.0,
            vad_window_size: 512,
            vad_buffer_seconds: 60.0,
            partial_interval: Duration::from_millis(300),
            early_commit: true,
        }
    }
}

impl AsrConfig {
    /// Build a config from the three required model paths, with defaults for
    /// everything else.
    pub fn new(
        sense_voice_model: impl Into<PathBuf>,
        tokens: impl Into<PathBuf>,
        silero_vad_model: impl Into<PathBuf>,
    ) -> Self {
        Self {
            sense_voice_model: sense_voice_model.into(),
            tokens: tokens.into(),
            silero_vad_model: silero_vad_model.into(),
            ..Default::default()
        }
    }

    /// Audio kept before the VAD reports speech: its `min_speech_duration`
    /// detection delay plus 0.3 s of margin.
    pub(crate) fn preroll_samples(&self) -> usize {
        ((self.min_speech_duration.max(0.0) + 0.3) * SAMPLE_RATE as f32) as usize
    }

    /// `max_decode_secs` in samples; `None` when unlimited or not positive.
    pub(crate) fn max_decode_samples(&self) -> Option<usize> {
        self.max_decode_secs
            .filter(|s| *s > 0.0)
            .map(|s| (s * SAMPLE_RATE as f32) as usize)
    }

    /// `commit_overlap` in samples (negative counts as zero), at most a quarter
    /// of `max_decode_secs` so a decode always has room for new audio.
    pub(crate) fn overlap_samples(&self) -> usize {
        let overlap = (self.commit_overlap.max(0.0) * SAMPLE_RATE as f32) as usize;
        match self.max_decode_samples() {
            Some(max) => overlap.min(max / 4),
            None => overlap,
        }
    }

    /// Number of samples between interim re-decodes, derived from
    /// `partial_interval` at [`SAMPLE_RATE`] (at least one VAD window).
    pub(crate) fn partial_interval_samples(&self) -> usize {
        let by_time = (self.partial_interval.as_secs_f32() * SAMPLE_RATE as f32) as usize;
        by_time.max(self.vad_window_size.max(1) as usize)
    }
}

/// A transcription event emitted by [`AsrEngine`].
#[derive(Debug, Clone, PartialEq)]
pub enum Transcript {
    /// Interim text for the utterance currently being spoken. Each `Partial`
    /// *replaces* the previous one with the same `segment` and may change
    /// (correct itself) as more audio arrives. Not yet committed.
    Partial {
        /// Utterance id, shared with the `Final` that later commits it.
        segment: u64,
        /// Best-effort transcription of the audio heard so far.
        text: String,
    },
    /// A finalized utterance produced from a completed VAD segment. The text is
    /// stable and replaces every `Partial` with the same `segment`; use
    /// [`split_sentences`] to show it as short sentences.
    Final {
        /// Utterance id; increases by one per finalized VAD segment.
        segment: u64,
        /// The finalized transcription.
        text: String,
        /// Segment start time in seconds, relative to the start of the stream.
        start: f32,
        /// Segment duration in seconds.
        duration: f32,
    },
}

impl Transcript {
    /// The transcribed text, regardless of variant.
    pub fn text(&self) -> &str {
        match self {
            Transcript::Partial { text, .. } | Transcript::Final { text, .. } => text,
        }
    }

    /// The utterance id, regardless of variant.
    pub fn segment(&self) -> u64 {
        match self {
            Transcript::Partial { segment, .. } | Transcript::Final { segment, .. } => *segment,
        }
    }

    /// Whether this is a committed [`Transcript::Final`].
    pub fn is_final(&self) -> bool {
        matches!(self, Transcript::Final { .. })
    }
}

/// Split finalized text into sentences after `。！？!?…`, and after `.` when
/// followed by whitespace or the end (so `3.5` stays whole). Each sentence
/// gets a share of `start..start + duration` proportional to its non-space
/// character count; text without a sentence end stays one sentence.
pub fn split_sentences(text: &str, start: f32, duration: f32) -> Vec<TranscriptionSegment> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        cur.push(c);
        let ends = match c {
            '。' | '！' | '？' | '!' | '?' | '…' => true,
            '.' => chars.peek().is_none_or(|n| n.is_whitespace()),
            _ => false,
        };
        // Keep runs like "？！" or "……" together.
        let next_ends = chars
            .peek()
            .is_some_and(|n| matches!(n, '。' | '！' | '？' | '!' | '?' | '…'));
        if ends && !next_ends {
            parts.push(std::mem::take(&mut cur));
        }
    }
    parts.push(cur);
    parts.retain(|p| !p.trim().is_empty());

    let weight = |p: &str| p.chars().filter(|c| !c.is_whitespace()).count().max(1) as f32;
    let total: f32 = parts.iter().map(|p| weight(p)).sum();
    let mut at = start;
    parts
        .into_iter()
        .map(|p| {
            let d = duration * weight(&p) / total;
            let seg = TranscriptionSegment {
                text: p.trim().to_string(),
                start: at,
                duration: d,
            };
            at += d;
            seg
        })
        .collect()
}

#[cfg(test)]
#[path = "lib_test.rs"]
mod lib_test;
