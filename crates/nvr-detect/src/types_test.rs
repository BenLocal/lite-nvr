use super::*;

#[test]
fn detection_serializes_to_documented_shape() {
    let d = Detection {
        class_id: 0,
        label: "person".to_string(),
        bbox: BBox {
            x1: 34.0,
            y1: 50.0,
            x2: 220.0,
            y2: 640.0,
        },
        confidence: 0.82,
    };
    let v = serde_json::to_value(&d).unwrap();
    assert_eq!(v["class_id"], 0);
    assert_eq!(v["label"], "person");
    assert_eq!(v["bbox"]["x1"], 34.0);
    assert_eq!(v["bbox"]["y2"], 640.0);
    // Float comparison with tolerance due to f32 serialization
    assert!((v["confidence"].as_f64().unwrap() - 0.82).abs() < 0.001);
}

#[test]
fn model_result_roundtrips() {
    let m = ModelResult {
        name: "yolov8n".to_string(),
        infer_ms: 12.3,
        detections: vec![],
        error: None,
    };
    let s = serde_json::to_string(&m).unwrap();
    let back: ModelResult = serde_json::from_str(&s).unwrap();
    assert_eq!(back.name, "yolov8n");
    assert_eq!(back.detections.len(), 0);
    assert!(back.error.is_none());
}
