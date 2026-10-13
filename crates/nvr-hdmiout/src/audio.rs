use crate::{Error, Result};

/// Signed 16-bit interleaved PCM, routed to an explicit ALSA playback device.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            device: "default".into(),
            sample_rate: 48000,
            channels: 2,
        }
    }
}

impl AudioConfig {
    fn validate(&self) -> Result<()> {
        if self.device.trim().is_empty()
            || self.device.as_bytes().contains(&0)
            || self.sample_rate == 0
            || !(1..=2).contains(&self.channels)
        {
            return Err(Error::InvalidAudio);
        }
        Ok(())
    }
}

/// Dedicated-thread playback with nonblocking ALSA I/O and bounded waits.
pub struct AudioOutput {
    #[cfg(target_os = "linux")]
    pcm: alsa::pcm::PCM,
    #[cfg(target_os = "linux")]
    channels: usize,
    #[cfg(target_os = "linux")]
    active: bool,
    #[cfg(not(target_os = "linux"))]
    _private: (),
}

impl AudioOutput {
    pub fn open(config: &AudioConfig) -> Result<Self> {
        config.validate()?;
        #[cfg(target_os = "linux")]
        {
            use alsa::{
                Direction, ValueOr,
                pcm::{Access, Format, HwParams, PCM},
            };
            let pcm = PCM::new(&config.device, Direction::Playback, true)?;
            {
                let params = HwParams::any(&pcm)?;
                params.set_access(Access::RWInterleaved)?;
                params.set_format(Format::s16())?;
                params.set_channels(config.channels)?;
                params.set_rate(config.sample_rate, ValueOr::Nearest)?;
                params.set_period_size_near(
                    i64::from((config.sample_rate / 100).max(1)),
                    ValueOr::Nearest,
                )?;
                params.set_buffer_size_near(i64::from((config.sample_rate / 10).max(1)))?;
                pcm.hw_params(&params)?;
            }
            Ok(Self {
                pcm,
                channels: usize::try_from(config.channels).map_err(|_| Error::InvalidAudio)?,
                active: true,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(Error::UnsupportedAudio)
        }
    }

    /// Write complete PCM frames. Handles partial writes and recovers underruns.
    pub fn write(&mut self, samples: &[i16]) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            use std::{
                io::ErrorKind,
                time::{Duration, Instant},
            };
            if !self.active {
                return Err(Error::Closed);
            }
            if !samples.len().is_multiple_of(self.channels) {
                return Err(Error::InvalidAudio);
            }
            let io = self.pcm.io_i16()?;
            let mut offset = 0;
            let mut progress = Instant::now();
            while offset < samples.len() {
                if progress.elapsed() >= Duration::from_secs(1) {
                    return Err(Error::AudioTimeout);
                }
                match io.writei(&samples[offset..]) {
                    Ok(frames) if frames > 0 => {
                        offset = frames
                            .checked_mul(self.channels)
                            .and_then(|n| offset.checked_add(n))
                            .filter(|n| *n <= samples.len())
                            .ok_or(Error::InvalidAudio)?;
                        progress = Instant::now();
                        continue;
                    }
                    Err(error)
                        if std::io::Error::from_raw_os_error(error.errno()).kind()
                            != ErrorKind::WouldBlock =>
                    {
                        self.pcm.try_recover(error, true)?;
                    }
                    _ => {}
                }
                self.pcm.wait(Some(100))?;
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = samples;
            Err(Error::UnsupportedAudio)
        }
    }

    /// Discard queued samples immediately; stopping must not drain old audio.
    pub fn close(&mut self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            if self.active {
                self.pcm.drop()?;
                self.active = false;
            }
        }
        Ok(())
    }
}

impl Drop for AudioOutput {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(test)]
#[path = "audio_test.rs"]
mod audio_test;
