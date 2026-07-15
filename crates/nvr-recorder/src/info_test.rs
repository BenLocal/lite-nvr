use chrono::DateTime;

use super::*;

#[test]
fn codec_name_resolves_known_ids() {
    assert_eq!(codec_name(ffmpeg_next::codec::Id::H264), "h264");
    assert_eq!(codec_name(ffmpeg_next::codec::Id::AAC), "aac");
}

#[test]
fn segment_info_serializes_expected_shape() {
    let s = SegmentInfo {
        path: "/tmp/rec.ts".into(),
        start_wall: DateTime::from_timestamp(1000, 0).unwrap(),
        end_wall: DateTime::from_timestamp(1010, 0).unwrap(),
        duration: 10.0,
        size_bytes: 123,
        video: Some(VideoMeta {
            codec: "h264".into(),
            width: 640,
            height: 360,
            fps: 25.0,
        }),
        audio: None,
    };
    let j = serde_json::to_value(&s).unwrap();
    assert_eq!(j["duration"], 10.0);
    assert_eq!(j["size_bytes"], 123);
    assert_eq!(j["video"]["codec"], "h264");
    assert_eq!(j["video"]["width"], 640);
    assert!(j["audio"].is_null());
}
