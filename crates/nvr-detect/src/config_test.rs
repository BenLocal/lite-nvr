use super::*;

#[test]
fn parses_manifest_with_defaults() {
    let json = r#"[
      { "name": "yolov8n", "model_file": "yolov8n.onnx", "version": 8.0 },
      { "name": "yolo11s", "model_file": "yolo11s.onnx", "input_size": 640,
        "conf": 0.3, "device": "cpu" }
    ]"#;
    let cfgs: Vec<DetectorConfig> = serde_json::from_str(json).unwrap();
    assert_eq!(cfgs.len(), 2);
    // Defaults applied on the first entry.
    assert_eq!(cfgs[0].input_size, 640);
    assert_eq!(cfgs[0].conf, 0.25);
    assert_eq!(cfgs[0].device, "cpu");
    assert!(cfgs[0].class_names.is_empty());
    assert_eq!(cfgs[0].version, Some(8.0));
    // Explicit values on the second.
    assert_eq!(cfgs[1].conf, 0.3);
}
