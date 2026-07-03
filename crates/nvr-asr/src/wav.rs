//! Minimal WAV loading for the demo: decode to mono `f32` in `[-1, 1]`.
//!
//! Kept dependency-light (pure-Rust `hound`) so the demo runs without the media
//! pipeline. Real deployments feed [`AsrEngine`](crate::AsrEngine) from a live
//! source instead.

use std::path::Path;

use anyhow::Context;

use crate::SAMPLE_RATE;

/// Mono audio decoded from a WAV file.
#[derive(Debug, Clone)]
pub struct WavAudio {
    /// The file's sample rate, in Hz.
    pub sample_rate: u32,
    /// Mono samples in `[-1, 1]`.
    pub samples: Vec<f32>,
}

impl WavAudio {
    /// Decode `path` to mono `f32`, downmixing multi-channel audio by averaging.
    /// Integer PCM (8/16/24/32-bit) and float WAVs are both supported.
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let mut reader = hound::WavReader::open(path)
            .with_context(|| format!("open wav {}", path.display()))?;
        let spec = reader.spec();

        let interleaved: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<Result<_, _>>()
                .context("read float wav samples")?,
            hound::SampleFormat::Int => {
                // Normalize by the format's full-scale value for the bit depth.
                let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / scale))
                    .collect::<Result<_, _>>()
                    .context("read int wav samples")?
            }
        };

        Ok(Self {
            sample_rate: spec.sample_rate,
            samples: downmix_to_mono(&interleaved, spec.channels.max(1)),
        })
    }
}

/// Load a WAV and require it to already be 16 kHz mono-decodable. Returns the
/// mono `f32` samples. Errors (with a conversion hint) on any other rate, since
/// SenseVoice/Silero are trained at 16 kHz and this crate does no resampling.
pub fn load_wav_16k_mono(path: impl AsRef<Path>) -> anyhow::Result<Vec<f32>> {
    let path = path.as_ref();
    let audio = WavAudio::load(path)?;
    anyhow::ensure!(
        audio.sample_rate == SAMPLE_RATE as u32,
        "wav {} is {} Hz, need {} Hz; convert first, e.g. `ffmpeg -i in.wav -ar {} -ac 1 out.wav`",
        path.display(),
        audio.sample_rate,
        SAMPLE_RATE,
        SAMPLE_RATE,
    );
    Ok(audio.samples)
}

/// Average interleaved channels down to a single mono track.
pub(crate) fn downmix_to_mono(interleaved: &[f32], channels: u16) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    if channels == 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

#[cfg(test)]
#[path = "wav_test.rs"]
mod wav_test;
