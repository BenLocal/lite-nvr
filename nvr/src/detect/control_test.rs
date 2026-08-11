// control_test is a child module of `control` (this file), so `super` == control.
use super::{
    MAX_DETECT_MODEL_NAME_CHARS, MAX_DETECT_MODELS, normalize_detect_config, should_auto_start,
    should_keep_retrying, validate_detect_config,
};
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

#[test]
fn no_auto_start_for_non_pipe_inputs() {
    for input_type in ["onvif", "stream", "xiaomi"] {
        assert!(
            !should_auto_start(Some(&cfg(true)), input_type),
            "{input_type} does not expose a subscribable Pipe"
        );
    }
}

#[test]
fn detect_config_rejects_confidence_outside_unit_interval() {
    assert!(
        validate_detect_config(Some(&DetectConfig {
            min_confidence: 1.1,
            ..cfg(true)
        }))
        .is_err()
    );
    assert!(
        validate_detect_config(Some(&DetectConfig {
            min_confidence: -0.1,
            ..cfg(true)
        }))
        .is_err()
    );
}

#[test]
fn detect_config_rejects_empty_model_names() {
    assert!(
        validate_detect_config(Some(&DetectConfig {
            models: vec!["".to_string()],
            ..cfg(true)
        }))
        .is_err()
    );
}

#[test]
fn detect_config_accepts_default_and_bounded_values() {
    assert!(validate_detect_config(Some(&cfg(true))).is_ok());
    assert!(
        validate_detect_config(Some(&DetectConfig {
            sample_every_ms: super::MAX_DETECT_SAMPLE_INTERVAL_MS,
            min_confidence: 1.0,
            ..cfg(true)
        }))
        .is_ok()
    );
    assert!(
        validate_detect_config(Some(&DetectConfig {
            sample_every_ms: super::MAX_DETECT_SAMPLE_INTERVAL_MS + 1,
            ..cfg(true)
        }))
        .is_err()
    );
}

#[test]
fn detect_config_accepts_unknown_model_for_backward_compatibility() {
    assert!(
        validate_detect_config(Some(&DetectConfig {
            models: vec!["missing".to_string()],
            ..cfg(true)
        }))
        .is_ok()
    );
}

#[test]
fn detect_config_trims_and_deduplicates_model_names() {
    let normalized = normalize_detect_config(Some(&DetectConfig {
        models: vec![
            " person ".to_string(),
            "vehicle".to_string(),
            "person".to_string(),
        ],
        ..cfg(true)
    }))
    .expect("valid config")
    .expect("detection config");

    assert_eq!(normalized.models, vec!["person", "vehicle"]);
}

#[test]
fn detect_config_bounds_model_count_and_name_length() {
    assert!(
        normalize_detect_config(Some(&DetectConfig {
            models: vec!["model".to_string(); MAX_DETECT_MODELS + 1],
            ..cfg(true)
        }))
        .is_err()
    );
    assert!(
        normalize_detect_config(Some(&DetectConfig {
            models: vec!["x".repeat(MAX_DETECT_MODEL_NAME_CHARS + 1)],
            ..cfg(true)
        }))
        .is_err()
    );
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
