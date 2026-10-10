#![allow(dead_code)]

/// Registers FFmpeg components (format, device, etc.). Call once at startup
/// before using device inputs like x11grab or v4l2.
pub fn init() -> anyhow::Result<()> {
    ffmpeg_next::init().map_err(|e| anyhow::anyhow!("ffmpeg_next init: {}", e))?;
    #[cfg(feature = "rockchip")]
    anyhow::ensure!(
        ffmpeg_next::encoder::find_by_name("h264_rkmpp").is_some()
            && ffmpeg_next::decoder::find_by_name("h264_rkmpp").is_some(),
        "rockchip feature requires RK FFmpeg with h264_rkmpp encoder and decoder; check FFMPEG_DIR and LD_LIBRARY_PATH"
    );
    Ok(())
}

/// Shared test fixture: `e2e/test.mp4` at the workspace root
/// (320x240, ~5s @ 10fps, 50 video frames + AAC). Works regardless of cwd.
#[cfg(test)]
pub(crate) fn test_mp4_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../e2e/test.mp4")
}

pub mod audio_mixer;
pub mod bsf;
pub mod bus;
pub mod decoder;
pub mod device;
pub mod encoder;
pub mod frame;
pub mod hw;
pub mod input;
pub mod metadata;
pub mod output;
pub mod packet;
pub mod scaler;
pub mod sink;
pub mod stream;
