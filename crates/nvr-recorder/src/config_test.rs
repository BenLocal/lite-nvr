use super::*;

#[test]
fn defaults_are_documented() {
    let c = RecorderConfig::new("rtsp://x", "/tmp/out");
    assert_eq!(c.url, "rtsp://x");
    assert_eq!(c.transport, RtspTransport::Tcp);
    assert_eq!(c.tracks, TrackSelect::Both);
    assert_eq!(c.segment_time, Duration::from_secs(60));
    assert!(!c.align_to_wall_clock);
    assert_eq!(c.container, Container::Ts);
    assert_eq!(c.filename_pattern, "rec_%Y%m%d_%H%M%S");
    assert_eq!(c.open_timeout, Duration::from_secs(5));
    assert_eq!(c.reconnect.max_retries, None);
    assert_eq!(c.reconnect.base_delay, Duration::from_secs(1));
    assert_eq!(c.reconnect.max_delay, Duration::from_secs(16));
}

#[test]
fn container_maps_to_muxer_and_extension() {
    assert_eq!(Container::Ts.muxer_name(), "mpegts");
    assert_eq!(Container::Ts.extension(), "ts");
    assert_eq!(Container::Mp4.muxer_name(), "mp4");
    assert_eq!(Container::Mkv.muxer_name(), "matroska");
}
