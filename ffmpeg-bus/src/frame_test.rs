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

