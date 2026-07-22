use super::hub::DetectHub;
use super::result::FrameResult;
use nvr_detect::ModelResult;

#[test]
fn store_and_latest_roundtrip_and_register_is_idempotent() {
    // A fresh, un-init'd hub instance for isolated testing.
    let hub = DetectHub::new_for_test(vec![], std::path::PathBuf::from("."), 500);

    assert!(hub.latest("cam1").is_none());
    let fr = FrameResult {
        ts: 42,
        frame_w: 1920,
        frame_h: 1080,
        models: vec![ModelResult {
            name: "m".into(),
            infer_ms: 1.0,
            detections: vec![],
            error: None,
        }],
    };
    hub.store("cam1", fr.clone());
    let got = hub.latest("cam1").expect("stored");
    assert_eq!(got.ts, 42);
    assert_eq!(got.models.len(), 1);

    let tok = tokio_util::sync::CancellationToken::new();
    assert!(hub.register("cam1", tok.clone()));
    assert!(!hub.register("cam1", tok.clone())); // already running
    assert!(hub.is_running("cam1"));
    assert!(hub.unregister("cam1"));
    assert!(!hub.is_running("cam1"));
}
