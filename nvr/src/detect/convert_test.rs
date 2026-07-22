use super::*;
use ffmpeg_bus::frame::RawVideoFrame;

#[test]
fn converts_yuv420p_frame_to_packed_rgb24() {
    // A 4x2 YUV420P frame (planes are allocated/zeroed by ffmpeg).
    let src = ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::YUV420P, 4, 2);
    let frame = RawVideoFrame::from(src);

    let (rgb, w, h) = to_rgb(&frame).expect("convert");
    assert_eq!(w, 4);
    assert_eq!(h, 2);
    // Tightly packed RGB24: exactly w*h*3 bytes, no row padding.
    assert_eq!(rgb.len(), (4 * 2 * 3) as usize);
}
