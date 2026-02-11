#![allow(dead_code)]

/// Registers FFmpeg components (format, device, etc.). Call once at startup
/// before using device inputs like x11grab or v4l2.
pub fn init() -> anyhow::Result<()> {
    ffmpeg_next::init().map_err(|e| anyhow::anyhow!("ffmpeg_next init: {}", e))
}

pub mod audio_mixer;
pub mod bsf;
pub mod bus;
pub mod decoder;
pub mod device;
pub mod encoder;
pub mod frame;
pub mod input;
pub mod metadata;
pub mod output;
pub mod packet;
pub mod scaler;
pub mod sink;
pub mod stream;
