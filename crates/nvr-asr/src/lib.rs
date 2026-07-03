//! Real-time speech-to-text using an **offline** ASR model to simulate
//! streaming, with correction.
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
//! WAV-file driver that simulates a live stream.
//!
//! [Silero VAD]: https://github.com/snakers4/silero-vad
//! [SenseVoice]: https://github.com/FunAudioLLM/SenseVoice

use std::{path::PathBuf, time::Duration};

mod engine;
mod wav;

pub use engine::AsrEngine;
pub use wav::{WavAudio, load_wav_16k_mono};

/// The canonical sample rate SenseVoice + Silero VAD expect.
pub const SAMPLE_RATE: i32 = 16_000;

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

    // --- recognizer options ---
    /// SenseVoice language: `auto`, `zh`, `en`, `ja`, `ko`, or `yue`.
    pub language: String,
    /// Enable inverse text normalization (digits/punctuation in the output).
    pub use_itn: bool,
    /// Recognizer threads.
    pub num_threads: i32,
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
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            sense_voice_model: PathBuf::new(),
            tokens: PathBuf::new(),
            silero_vad_model: PathBuf::new(),
            language: "auto".to_string(),
            use_itn: true,
            num_threads: 2,
            debug: false,
            vad_threshold: 0.5,
            min_silence_duration: 0.25,
            min_speech_duration: 0.25,
            max_speech_duration: 20.0,
            vad_window_size: 512,
            vad_buffer_seconds: 60.0,
            partial_interval: Duration::from_millis(300),
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
    /// *replaces* the previous one for the same utterance and may change
    /// (correct itself) as more audio arrives. Not yet committed.
    Partial {
        /// Best-effort transcription of the audio heard so far.
        text: String,
    },
    /// A finalized utterance produced from a completed VAD segment. The text is
    /// stable; render it on its own line and clear any pending partial.
    Final {
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
            Transcript::Partial { text } | Transcript::Final { text, .. } => text,
        }
    }

    /// Whether this is a committed [`Transcript::Final`].
    pub fn is_final(&self) -> bool {
        matches!(self, Transcript::Final { .. })
    }
}

#[cfg(test)]
#[path = "lib_test.rs"]
mod lib_test;
