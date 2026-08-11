use super::hub::DetectHub;
use super::result::FrameResult;
use nvr_detect::{Detection, Detector, ModelResult};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

struct NamedDetector(&'static str);

impl Detector for NamedDetector {
    fn name(&self) -> &str {
        self.0
    }

    fn detect(&self, _rgb: &[u8], _width: u32, _height: u32) -> anyhow::Result<Vec<Detection>> {
        Ok(vec![])
    }
}

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
fn unknown_only_model_selection_falls_back_to_all_models() {
    let hub = DetectHub::new_for_test(vec![], std::path::PathBuf::from("."), 500);
    let all: Vec<Arc<dyn Detector>> = vec![Arc::new(NamedDetector("current"))];

    let selected = hub.detectors_named(&all, &Some(vec!["retired".to_string()]));

    assert_eq!(
        selected
            .iter()
            .map(|detector| detector.name())
            .collect::<Vec<_>>(),
        vec!["current"]
    );
}

#[test]
fn mixed_known_and_unknown_model_selection_runs_the_known_subset() {
    let hub = DetectHub::new_for_test(vec![], std::path::PathBuf::from("."), 500);
    let all: Vec<Arc<dyn Detector>> = vec![
        Arc::new(NamedDetector("person")),
        Arc::new(NamedDetector("vehicle")),
    ];

    let selected = hub.detectors_named(
        &all,
        &Some(vec!["retired".to_string(), "vehicle".to_string()]),
    );

    assert_eq!(
        selected
            .iter()
            .map(|detector| detector.name())
            .collect::<Vec<_>>(),
        vec!["vehicle"]
    );
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
fn replacing_auto_start_cancels_the_previous_generation() {
    let hub = DetectHub::new_for_test(vec![], std::path::PathBuf::new(), 500);
    let (old_generation, old_token) = hub.begin_auto_start("cam1");
    let (new_generation, _new_token) = hub.begin_auto_start("cam1");

    assert!(old_token.is_cancelled());
    assert!(
        hub.register_auto_start("cam1", old_generation, CancellationToken::new())
            .is_none()
    );
    assert!(
        hub.register_auto_start("cam1", new_generation, CancellationToken::new())
            .is_some()
    );
    hub.unregister("cam1");
}

#[test]
fn stop_cancels_a_pending_auto_start_before_it_can_register() {
    let hub = DetectHub::new_for_test(vec![], std::path::PathBuf::new(), 500);
    let (generation, pending) = hub.begin_auto_start("cam1");

    assert!(hub.stop("cam1"));
    assert!(pending.is_cancelled());
    assert!(
        hub.register_auto_start("cam1", generation, CancellationToken::new())
            .is_none()
    );
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

#[test]
fn a_stale_manual_lease_cannot_stop_the_tap_that_replaced_it() {
    let hub = DetectHub::new_for_test(vec![], std::path::PathBuf::from("."), 500);

    let old_cancel = CancellationToken::new();
    let old_epoch = hub.register("cam1", old_cancel).expect("register old tap");
    assert!(hub.unregister("cam1"));

    let replacement_cancel = CancellationToken::new();
    let replacement_epoch = hub
        .register("cam1", replacement_cancel.clone())
        .expect("register replacement tap");
    assert_ne!(old_epoch, replacement_epoch);

    assert!(!hub.stop_tap("cam1", old_epoch));
    assert!(hub.is_running("cam1"));
    assert!(!replacement_cancel.is_cancelled());

    assert!(hub.stop_tap("cam1", replacement_epoch));
    assert!(replacement_cancel.is_cancelled());
    assert!(!hub.is_running("cam1"));
}
