use super::*;

#[test]
fn traffic_samples_are_passive_and_idle_rates_expire() {
    let traffic = Traffic::default();
    let start = Instant::now();
    traffic.record_at(100, start);
    traffic.record_at(200, start + Duration::from_secs(1));
    let now = start + Duration::from_secs(1);
    let first = traffic.snapshot_at(now);
    let second = traffic.snapshot_at(now);
    assert_eq!(first.bytes_total, 300);
    assert_eq!(first.packets_total, 2);
    assert_eq!(first.bytes_per_second, 300.0);
    assert_eq!(second.bytes_per_second, first.bytes_per_second);
    assert_eq!(first.packets_per_second, 2.0);
    assert_eq!(
        traffic
            .snapshot_at(start + Duration::from_secs(3))
            .bytes_per_second,
        0.0
    );
    traffic.record_at(20, start + Duration::from_secs(4));
    traffic.record_at(30, start + Duration::from_secs(5));
    assert_eq!(
        traffic
            .snapshot_at(start + Duration::from_secs(5))
            .bytes_per_second,
        50.0
    );
    assert_eq!(
        traffic
            .snapshot_at(start + Duration::from_secs(5))
            .bytes_total,
        350
    );
}

#[test]
fn codec_probe_reports_runtime_hardware_downgrade() {
    let stream = AvStream::new(
        3,
        ffmpeg_next::codec::Parameters::new(),
        ffmpeg_next::Rational(1, 1000),
        ffmpeg_next::Rational(0, 0),
    );
    let probe = CodecProbe::default();
    probe.update(&stream, "h264_rkmpp", true);
    assert!(probe.snapshot(true).hardware);
    probe.update(&stream, "h264", false);
    let snapshot = probe.snapshot(false);
    assert!(!snapshot.hardware);
    assert!(!snapshot.running);
    assert_eq!(snapshot.codec, "h264");
    assert_eq!(snapshot.stream_index, 3);
    assert!(
        StreamMetrics::from_stream(&stream, TrafficMetrics::default())
            .fps
            .is_none()
    );
}

#[test]
fn codec_probe_counts_completed_frames_and_encoded_bytes() {
    let probe = CodecProbe::default();
    probe.record(100);
    probe.record(200);
    let snapshot = probe.snapshot(true);
    assert_eq!(snapshot.traffic.packets_total, 2);
    assert_eq!(snapshot.traffic.bytes_total, 300);
    let decoder = CodecProbe::default();
    decoder.record(0);
    assert_eq!(decoder.snapshot(true).traffic.packets_total, 1);
    assert_eq!(decoder.snapshot(true).traffic.bytes_total, 0);
}
