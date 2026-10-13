use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MixMode {
    #[default]
    Program,
    AddOriginal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioCodec {
    #[default]
    Aac,
    Opus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelMap {
    #[default]
    Stereo,
    Swap,
    LeftToBoth,
    RightToBoth,
}

impl ChannelMap {
    pub(crate) fn value(self) -> u8 {
        match self {
            Self::Stereo => 0,
            Self::Swap => 1,
            Self::LeftToBoth => 2,
            Self::RightToBoth => 3,
        }
    }
    pub(crate) fn from_value(value: u8) -> Self {
        match value {
            1 => Self::Swap,
            2 => Self::LeftToBoth,
            3 => Self::RightToBoth,
            _ => Self::Stereo,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioProfile {
    pub codec: AudioCodec,
    pub sample_rate: u32,
    pub channels: u32,
    pub bitrate: u64,
}

impl Default for AudioProfile {
    fn default() -> Self {
        Self {
            codec: AudioCodec::Aac,
            sample_rate: 48_000,
            channels: 2,
            bitrate: 128_000,
        }
    }
}

impl AudioProfile {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.channels == 2, "节目音频必须保留左右独立双声道");
        anyhow::ensure!(
            (16_000..=512_000).contains(&self.bitrate),
            "音频码率必须为 16～512 Kbps"
        );
        let rates: &[u32] = match self.codec {
            AudioCodec::Aac => &[
                8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000,
            ],
            AudioCodec::Opus => &[8000, 12000, 16000, 24000, 48000],
        };
        anyhow::ensure!(rates.contains(&self.sample_rate), "编码不支持所选采样率");
        Ok(())
    }

    pub(crate) fn encoder_name(&self) -> &'static str {
        match self.codec {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "libopus",
        }
    }
}

#[cfg(test)]
#[path = "profile_test.rs"]
mod profile_test;
