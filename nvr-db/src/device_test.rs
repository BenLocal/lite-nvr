// device_test is a child module of `device` (this file), so `super` == device.
use super::{DetectConfig, DeviceConfig, DeviceInfo};

#[test]
fn device_without_config_deserializes_to_default() {
    // A row written before this field exists must still load, with detection off.
    let json = r#"{
        "id":"cam1","name":"Cam 1","input_type":"rtsp","input_value":"rtsp://x",
        "description":"","include_audio":false,"record":true,
        "created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"
    }"#;
    let dev: DeviceInfo = serde_json::from_str(json).expect("legacy row must parse");
    assert!(dev.config.detect.is_none());
}

#[test]
fn detect_config_round_trips() {
    let cfg = DeviceConfig {
        detect: Some(DetectConfig {
            enabled: true,
            models: vec!["yolov8n".to_string()],
            sample_every_ms: 500,
            min_confidence: 0.4,
        }),
    };
    let s = serde_json::to_string(&cfg).unwrap();
    let back: DeviceConfig = serde_json::from_str(&s).unwrap();
    let d = back.detect.expect("detect present");
    assert!(d.enabled);
    assert_eq!(d.models, vec!["yolov8n".to_string()]);
    assert_eq!(d.sample_every_ms, 500);
    assert!((d.min_confidence - 0.4).abs() < 1e-6);
}
