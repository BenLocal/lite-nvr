// control_test is a child module of `control` (this file), so `super` == control.
use super::{should_auto_start, should_keep_retrying};
use nvr_db::device::{DetectConfig, DeviceConfig, DeviceInfo};

fn cfg(enabled: bool) -> DetectConfig {
    DetectConfig {
        enabled,
        models: vec![],
        sample_every_ms: 0,
        min_confidence: 0.0,
    }
}

#[test]
fn auto_start_when_enabled_and_pipe_backed() {
    assert!(should_auto_start(Some(&cfg(true)), "rtsp"));
}

#[test]
fn no_auto_start_when_disabled() {
    assert!(!should_auto_start(Some(&cfg(false)), "rtsp"));
}

#[test]
fn no_auto_start_when_absent() {
    assert!(!should_auto_start(None, "rtsp"));
}

#[test]
fn no_auto_start_for_gb28181() {
    assert!(!should_auto_start(Some(&cfg(true)), "gb28181"));
}

fn device(detect: Option<DetectConfig>) -> DeviceInfo {
    let now = chrono::Utc::now();
    DeviceInfo {
        id: "cam1".to_string(),
        name: "cam1".to_string(),
        input_type: "rtsp".to_string(),
        input_value: "rtsp://x/y".to_string(),
        description: String::new(),
        include_audio: false,
        record: true,
        config: DeviceConfig { detect },
        created_at: now,
        updated_at: now,
    }
}

// A fresh pipe publishes its bus asynchronously, so auto-start retries until
// `subscribe_video` succeeds. Between attempts it re-reads the device, and these
// are the conditions under which a pending retry must abandon its attempt —
// otherwise it would start a tap for a device the user just turned off.

#[test]
fn retry_continues_while_detection_is_still_enabled() {
    assert!(should_keep_retrying(Some(&device(Some(cfg(true))))));
}

#[test]
fn retry_stops_when_detection_was_disabled_mid_wait() {
    assert!(!should_keep_retrying(Some(&device(Some(cfg(false))))));
}

#[test]
fn retry_stops_when_the_device_was_removed_mid_wait() {
    assert!(!should_keep_retrying(None));
}
