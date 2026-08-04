// control_test is a child module of `control` (this file), so `super` == control.
use super::should_auto_start;
use nvr_db::device::DetectConfig;

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
