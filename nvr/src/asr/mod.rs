//! Real-time ASR for live pipes: taps decoded audio, transcribes via `nvr-asr`,
//! and emits transcripts over Socket.IO. Opt-in per pipe.

pub mod resample;

#[cfg(test)]
#[path = "resample_test.rs"]
mod resample_test;
