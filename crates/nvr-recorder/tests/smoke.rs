use std::time::Duration;

use nvr_recorder::{Container, Recorder, RecorderConfig, TrackSelect};
use tokio_util::sync::CancellationToken;

/// Record a real RTSP source for ~12s at 4s segments and assert we produced
/// at least two segments with monotonic start times. Requires `RTSP_TEST_URL`.
///
/// Run with, e.g.:
///   RTSP_TEST_URL=rtsp://127.0.0.1:8554/stream \
///   LD_LIBRARY_PATH=$PWD/ffmpeg/lib \
///   cargo test -p nvr-recorder --test smoke -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn records_segments_from_live_rtsp() {
    let Ok(url) = std::env::var("RTSP_TEST_URL") else {
        eprintln!("RTSP_TEST_URL not set; skipping");
        return;
    };
    ffmpeg_bus::init().unwrap();

    let dir = std::env::temp_dir().join("nvr-recorder-smoke");
    let _ = std::fs::remove_dir_all(&dir);

    let mut config = RecorderConfig::new(url, &dir);
    config.tracks = TrackSelect::Both;
    config.container = Container::Ts;
    config.segment_time = Duration::from_secs(4);

    let (recorder, mut rx) = Recorder::new(config);
    let cancel = CancellationToken::new();
    let run_cancel = cancel.clone();
    let handle = tokio::spawn(async move { recorder.run(run_cancel).await });

    let mut segments = Vec::new();
    let deadline = tokio::time::sleep(Duration::from_secs(12));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => break,
            Some(info) = rx.recv() => segments.push(info),
        }
    }
    cancel.cancel();
    // Drain any final segment emitted during shutdown.
    while let Ok(Some(info)) = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
        segments.push(info);
    }
    let _ = handle.await;

    assert!(
        segments.len() >= 2,
        "expected >= 2 segments, got {}",
        segments.len()
    );
    for w in segments.windows(2) {
        assert!(
            w[1].start_wall >= w[0].start_wall,
            "segment start times must be monotonic"
        );
    }
    for s in &segments {
        let meta = std::fs::metadata(&s.path).expect("segment file exists");
        assert!(meta.len() > 0, "segment file must be non-empty");
    }
}
