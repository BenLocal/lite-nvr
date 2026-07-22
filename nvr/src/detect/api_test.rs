use super::StartBody;

#[test]
fn start_body_defaults_models_to_none() {
    // Empty body → run all configured models.
    let b: StartBody = serde_json::from_str("{}").unwrap();
    assert!(b.models.is_none());

    let b: StartBody = serde_json::from_str(r#"{"models":["yolov8n"]}"#).unwrap();
    assert_eq!(b.models.unwrap(), vec!["yolov8n".to_string()]);
}
