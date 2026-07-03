//! The [`AsrEngine`]: an offline recognizer driven by a VAD to simulate
//! streaming with correction. See the crate docs for the overall approach.

use anyhow::Context;
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, VadModelConfig, VoiceActivityDetector,
};

use crate::{AsrConfig, SAMPLE_RATE, Transcript};

/// Real-time speech-to-text engine. Feed it 16 kHz mono `f32` PCM and drain
/// [`Transcript`] events. Not `Sync`; drive it from a single task/thread.
pub struct AsrEngine {
    recognizer: OfflineRecognizer,
    vad: VoiceActivityDetector,
    config: AsrConfig,

    /// Samples not yet aligned to a full VAD window.
    pending: Vec<f32>,
    /// Audio accumulated for the currently-open utterance, used to produce
    /// interim `Partial`s (the VAD owns the authoritative segment for `Final`).
    speech_buf: Vec<f32>,
    /// Samples fed into `speech_buf` since the last emitted `Partial`.
    since_partial: usize,
    /// Total samples accepted so far (unused for timing — VAD reports segment
    /// starts — but handy for debugging/back-pressure).
    total_samples: u64,
}

impl AsrEngine {
    /// Build the recognizer + VAD from `config`. Fails if a model path is
    /// missing/unreadable or the native library could not create a component.
    pub fn new(config: AsrConfig) -> anyhow::Result<Self> {
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

        let recognizer = build_recognizer(&config)
            .context("failed to create SenseVoice offline recognizer (check model/tokens paths and native lib)")?;
        let vad = build_vad(&config)
            .context("failed to create Silero VAD (check silero_vad_model path and native lib)")?;

        Ok(Self {
            recognizer,
            vad,
            config,
            pending: Vec::new(),
            speech_buf: Vec::new(),
            since_partial: 0,
            total_samples: 0,
        })
    }

    /// Push mono 16 kHz `f32` samples and return any transcription events
    /// produced. May emit zero or more `Partial`s and `Final`s per call.
    pub fn accept(&mut self, samples: &[f32]) -> Vec<Transcript> {
        let mut out = Vec::new();
        self.total_samples += samples.len() as u64;
        self.pending.extend_from_slice(samples);

        let win = self.config.vad_window_size.max(1) as usize;
        let partial_every = self.config.partial_interval_samples();

        while self.pending.len() >= win {
            // Silero expects fixed-size windows; hand it exactly one at a time.
            let window: Vec<f32> = self.pending.drain(..win).collect();
            self.vad.accept_waveform(&window);

            // While speech is ongoing, grow the interim buffer and re-decode it
            // periodically so the caller sees a self-correcting live transcript.
            if self.vad.detected() {
                self.speech_buf.extend_from_slice(&window);
                self.since_partial += window.len();
                if self.since_partial >= partial_every
                    && let Some(text) = self.decode(&self.speech_buf)
                {
                    self.since_partial = 0;
                    out.push(Transcript::Partial { text });
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
                if let Some(text) = self.decode(samples) {
                    let sr = SAMPLE_RATE as f32;
                    out.push(Transcript::Final {
                        text,
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

    /// Run the offline recognizer over `samples`, returning the trimmed text if
    /// it is non-empty.
    fn decode(&self, samples: &[f32]) -> Option<String> {
        if samples.is_empty() {
            return None;
        }
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE, samples);
        self.recognizer.decode(&stream);
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
    c.model_config.sense_voice.model = Some(config.sense_voice_model.to_string_lossy().into_owned());
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
