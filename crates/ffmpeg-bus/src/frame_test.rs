use super::*;

#[test]
fn test_video_frame_pts_and_dts_ms() {
    let mut frame = VideoFrame::new_encoded(vec![1, 2, 3], 1920, 1080, 27);
    frame.pts = 90_000;
    frame.dts = 45_000;

    let tb = Rational(1, 90_000);
    assert_eq!(frame.pts_ms(tb), 1000.0);
    assert_eq!(frame.dts_ms(tb), 500.0);
}

#[test]
fn test_video_frame_display_contains_core_fields() {
    let frame = VideoFrame::new(vec![1, 2, 3, 4], 640, 360, 0, 10, 8, true, 27);
    let s = frame.to_string();
    assert!(s.contains("data_len: 4"));
    assert!(s.contains("width: 640"));
    assert!(s.contains("height: 360"));
    assert!(s.contains("pts: 10"));
}

#[test]
fn test_packet_to_raw_video_frame_rejects_invalid_dimensions() {
    let packet = ffmpeg_next::codec::packet::Packet::empty();
    let raw = RawPacket::from((packet, Rational(1, 1000)));

    let err = packet_to_raw_video_frame(raw, 0, 1080, ffmpeg_next::format::Pixel::YUV420P).err();
    assert!(err.is_some());
}

#[test]
fn test_packet_to_raw_video_frame_rejects_invalid_pixel_format() {
    let packet = ffmpeg_next::codec::packet::Packet::empty();
    let raw = RawPacket::from((packet, Rational(1, 1000)));

    let err = packet_to_raw_video_frame(raw, 1280, 720, ffmpeg_next::format::Pixel::None).err();
    assert!(err.is_some());
}

#[test]
fn test_video_frame_try_from_audio_returns_error() {
    let mut audio = ffmpeg_next::frame::Audio::empty();
    audio.set_pts(Some(123));
    let raw = RawFrame::Audio(audio.into());
    let result = VideoFrame::try_from(raw);
    assert!(result.is_err());
}

#[test]
fn raw_video_frame_exposes_inner_via_as_video() {
    use ffmpeg_next::frame::Video;
    let src = Video::new(ffmpeg_next::format::Pixel::RGB24, 4, 2);
    let rvf = super::RawVideoFrame::from(src);
    let inner = rvf.as_video();
    assert_eq!(inner.width(), 4);
    assert_eq!(inner.height(), 2);
    assert_eq!(inner.format(), ffmpeg_next::format::Pixel::RGB24);
}

/// Changing a shared frame's properties must not copy its pixels: the
/// property copy references the same buffers, and the original is untouched.
#[test]
fn test_set_pts_shares_pixel_buffers() {
    let mut src = ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::YUV420P, 64, 48);
    src.set_pts(Some(7));
    let original = RawVideoFrame::from(src);
    let mut shared = original.clone();

    shared.set_pts(Some(42));

    assert_eq!(shared.pts(), Some(42));
    assert_eq!(original.pts(), Some(7), "original frame must be untouched");
    assert_eq!(
        shared.as_video().data(0).as_ptr(),
        original.as_video().data(0).as_ptr(),
        "set_pts must reference the same pixel buffer, not copy it"
    );
}

/// A Raw output frame carries all planes, packed without row padding, so a
/// consumer can rebuild the picture from width/height/format alone.
#[test]
fn test_video_frame_from_raw_packs_all_planes() -> anyhow::Result<()> {
    use ffmpeg_next::format::Pixel;
    // 50 is not a multiple of the 32/64-byte row alignment: rows are padded.
    let mut src = ffmpeg_next::frame::Video::new(Pixel::YUV420P, 50, 30);
    assert!(src.stride(0) > 50, "test needs a padded source");
    for plane in 0..3 {
        let fill = [0x10u8, 0x80, 0xf0][plane];
        src.data_mut(plane).fill(fill);
    }
    let vf = VideoFrame::try_from(RawFrame::Video(RawVideoFrame::from(src)))?;

    let (y, c) = (50 * 30, 25 * 15);
    assert_eq!(vf.data.len(), y + 2 * c, "Y + U + V, no padding");
    assert!(vf.data[..y].iter().all(|&b| b == 0x10), "Y plane");
    assert!(vf.data[y..y + c].iter().all(|&b| b == 0x80), "U plane");
    assert!(vf.data[y + c..].iter().all(|&b| b == 0xf0), "V plane");
    assert_eq!((vf.width, vf.height), (50, 30));
    Ok(())
}

#[test]
fn test_pixel_write_after_property_copy_keeps_original_unchanged() {
    let mut src = ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::YUV420P, 64, 48);
    src.data_mut(0).fill(11);
    let original = RawVideoFrame::from(src);
    let mut shared = original.clone();
    shared.set_pts(Some(42));
    shared.get_mut().data_mut(0)[0] = 99;
    assert_eq!(original.as_video().data(0)[0], 11);
    assert_eq!(shared.as_video().data(0)[0], 99);
}
