//! Decoded video of a device for in-process consumers. Each subscriber says
//! what it needs (see `FrameDemand`); the device's single shared decoder then
//! decodes keyframes only, or every frame once a transcode needs them.

use std::time::Duration;

use ffmpeg_bus::decoder::FrameDemand;
use ffmpeg_bus::frame::{RawFrame, RawFrameCmd, RawFrameReceiver, RawVideoFrame};
use tokio::sync::broadcast::error::RecvError;

pub(crate) async fn subscribe(
    device_id: &str,
    demand: FrameDemand,
) -> anyhow::Result<RawFrameReceiver> {
    let pipe = crate::manager::get_pipe(device_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("device has no running pipe"))?;
    pipe.subscribe_video_with(demand).await
}

/// The device's next decoded picture as JPEG. Unless the device is already
/// decoding every frame, only keyframes are decoded, so this may wait up to
/// one GOP.
pub(crate) async fn snapshot_jpeg(device_id: &str, wait: Duration) -> anyhow::Result<Vec<u8>> {
    let mut video = subscribe(device_id, FrameDemand::Image).await?;
    let frame = tokio::time::timeout(wait, next_video_frame(&mut video))
        .await
        .map_err(|_| anyhow::anyhow!("no picture within {}s", wait.as_secs()))??;
    // Leave the decoder before the CPU work, so it can stop if unused.
    drop(video);
    tokio::task::spawn_blocking(move || encode_jpeg(&frame)).await?
}

async fn next_video_frame(video: &mut RawFrameReceiver) -> anyhow::Result<RawVideoFrame> {
    loop {
        match video.recv().await {
            Ok(RawFrameCmd::Data(RawFrame::Video(frame))) => return Ok(frame),
            Ok(RawFrameCmd::Data(RawFrame::Audio(_))) | Err(RecvError::Lagged(_)) => {}
            Ok(RawFrameCmd::EOF) | Err(RecvError::Closed) => anyhow::bail!("video ended"),
        }
    }
}

pub(crate) fn encode_jpeg(frame: &RawVideoFrame) -> anyhow::Result<Vec<u8>> {
    let (rgb, width, height) = crate::detect::convert::to_rgb(frame)?;
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85).encode(
        &rgb,
        width,
        height,
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(jpeg)
}

#[cfg(test)]
#[path = "frames_test.rs"]
mod frames_test;
