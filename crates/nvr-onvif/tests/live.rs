use std::time::Duration;

use nvr_onvif::{OnvifCamera, OnvifConfig, PtzVelocity};

/// End-to-end against a real ONVIF camera / simulator. Run with, e.g.:
///   ONVIF_TEST_HOST=192.168.1.50 ONVIF_TEST_PORT=8000 \
///   ONVIF_TEST_USER=admin ONVIF_TEST_PASS=secret \
///   cargo test -p nvr-onvif --test live -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn connect_profiles_stream_uri_ptz() {
    let Ok(host) = std::env::var("ONVIF_TEST_HOST") else {
        eprintln!("ONVIF_TEST_HOST not set; skipping");
        return;
    };
    let cfg = OnvifConfig {
        host,
        port: std::env::var("ONVIF_TEST_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(80),
        username: std::env::var("ONVIF_TEST_USER").unwrap_or_default(),
        password: std::env::var("ONVIF_TEST_PASS").unwrap_or_default(),
        profile_token: None,
    };

    let cam = OnvifCamera::connect(&cfg).await.expect("connect");
    let info = cam.device_info().await.expect("device_info");
    println!(
        "device: {} {} fw {}",
        info.manufacturer, info.model, info.firmware
    );

    let profiles = cam.profiles().await.expect("profiles");
    assert!(!profiles.is_empty(), "expected at least one media profile");

    let uri = cam.stream_uri(None).await.expect("stream_uri");
    assert!(uri.starts_with("rtsp://"), "stream uri must be rtsp: {uri}");

    // PTZ is best-effort: some cameras have none. Don't fail the test on NoPtzService.
    if let Err(e) = cam.ptz_move(PtzVelocity::new(0.1, 0.0, 0.0)).await {
        println!("ptz_move skipped: {e}");
    } else {
        tokio::time::sleep(Duration::from_millis(300)).await;
        cam.ptz_stop().await.expect("ptz_stop");
    }
}
