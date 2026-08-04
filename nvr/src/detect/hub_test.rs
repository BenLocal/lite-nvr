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
    assert!(hub.register("cam1", tok.clone()).is_some());
    assert!(hub.register("cam1", tok.clone()).is_none()); // already running
    assert!(hub.is_running("cam1"));
    assert!(hub.unregister("cam1"));
    assert!(!hub.is_running("cam1"));
}

#[test]
fn finished_tap_frees_its_slot_so_detection_can_restart() {
    let hub = DetectHub::new_for_test(vec![], std::path::PathBuf::from("."), 500);
    let tok = tokio_util::sync::CancellationToken::new();

    let epoch = hub.register("cam1", tok).expect("register");
    assert!(hub.is_running("cam1"));

    // The device dropped its stream: the tap ends on its own, with nobody
    // calling `unregister`. Its slot must not linger as a zombie.
    assert!(hub.unregister_tap("cam1", epoch));
    assert!(!hub.is_running("cam1"));

    // A fresh tap can claim the pipe again.
    let tok2 = tokio_util::sync::CancellationToken::new();
    assert!(hub.register("cam1", tok2).is_some());
}

#[test]
fn a_stale_tap_cannot_evict_the_tap_that_replaced_it() {
    let hub = DetectHub::new_for_test(vec![], std::path::PathBuf::from("."), 500);

    let tok_a = tokio_util::sync::CancellationToken::new();
    let epoch_a = hub.register("cam1", tok_a).expect("register a");

    // Restart while tap A is still unwinding: stop it, then a new tap claims
    // the slot before A's cleanup runs.
    assert!(hub.unregister("cam1"));
    let tok_b = tokio_util::sync::CancellationToken::new();
    let epoch_b = hub.register("cam1", tok_b.clone()).expect("register b");
    assert_ne!(epoch_a, epoch_b);

    // A's late cleanup targets a generation that is no longer registered, so it
    // must be a no-op — otherwise it would silently kill the live tap B.
    assert!(!hub.unregister_tap("cam1", epoch_a));
    assert!(hub.is_running("cam1"));
    assert!(!tok_b.is_cancelled());

    // B's own cleanup still works.
    assert!(hub.unregister_tap("cam1", epoch_b));
    assert!(!hub.is_running("cam1"));
}
