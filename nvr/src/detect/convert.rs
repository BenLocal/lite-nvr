//! Convert a decoded video frame (any pixel format, e.g. YUV420P) into tightly-
//! packed RGB24 bytes for a detector. Reuses the ffmpeg-bus `Scaler`.

use ffmpeg_bus::frame::RawVideoFrame;
use ffmpeg_bus::scaler::Scaler;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::software::scaling::Context;
use ffmpeg_next::software::scaling::flag::Flags;

/// Returns `(rgb24_bytes, width, height)` with `rgb24_bytes.len() == w*h*3`
/// (row padding from the scaler's stride is removed).
pub fn to_rgb(frame: &RawVideoFrame) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let w = frame.width();
    let h = frame.height();
    if w == 0 || h == 0 {
        anyhow::bail!("zero-sized frame");
    }
    let src = frame.as_video();

    // Same Context::get arg order + Flags path the encoder uses (encoder.rs:531).
    let ctx = Context::get(src.format(), w, h, Pixel::RGB24, w, h, Flags::empty())?;
    let mut scaler = Scaler::new(ctx);

    // `Video::empty()` — the scaler allocates the destination (encoder idiom).
    let mut dst = ffmpeg_next::frame::Video::empty();
    scaler.run(src, &mut dst)?;

    // RGB24 has a single plane; stride may exceed w*3, so copy row by row.
    let stride = dst.stride(0);
    let row_bytes = (w as usize) * 3;
    let data = dst.data(0);
    let mut out = Vec::with_capacity(row_bytes * h as usize);
    for row in 0..h as usize {
        let start = row * stride;
        out.extend_from_slice(&data[start..start + row_bytes]);
    }
    Ok((out, w, h))
}

#[cfg(test)]
#[path = "convert_test.rs"]
mod convert_test;
