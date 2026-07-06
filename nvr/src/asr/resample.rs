//! Resample arbitrary decoded audio to 16 kHz mono f32 for the ASR engine.

use ffmpeg_next::ChannelLayout;
use ffmpeg_next::format::{Sample, sample::Type};
use ffmpeg_next::frame::Audio;
use ffmpeg_next::software::resampling::Context as Resampler;

const TARGET_RATE: u32 = 16_000;

/// Converts decoded audio frames to 16 kHz mono `f32` PCM. The swr context is
/// (re)built whenever the input format/layout/rate changes.
pub struct Pcm16kMono {
    swr: Option<Resampler>,
    in_fmt: Option<(Sample, ChannelLayout, u32)>,
}

impl Default for Pcm16kMono {
    fn default() -> Self {
        Self::new()
    }
}

impl Pcm16kMono {
    pub fn new() -> Self {
        Self {
            swr: None,
            in_fmt: None,
        }
    }

    /// Resample `frame` and return its 16 kHz mono samples.
    pub fn push(&mut self, frame: &Audio) -> anyhow::Result<Vec<f32>> {
        let rate = frame.rate();
        anyhow::ensure!(rate > 0, "audio frame has zero sample rate");
        let key = (frame.format(), frame.channel_layout(), rate);
        if self.in_fmt != Some(key) {
            self.swr = Some(Resampler::get(
                frame.format(),
                frame.channel_layout(),
                rate,
                Sample::F32(Type::Packed),
                ChannelLayout::MONO,
                TARGET_RATE,
            )?);
            self.in_fmt = Some(key);
        }
        let swr = self.swr.as_mut().expect("swr set above");
        // Upper bound on output samples; over-allocate to avoid the unsafe
        // swr_get_out_samples call. +1024 covers filter delay.
        let max_out = (frame.samples() as u64 * TARGET_RATE as u64 / rate as u64) as usize + 1024;
        let mut out = Audio::new(Sample::F32(Type::Packed), max_out, ChannelLayout::MONO);
        swr.run(frame, &mut out)?;
        let n = out.samples();
        Ok(out.plane::<f32>(0)[..n].to_vec())
    }
}
