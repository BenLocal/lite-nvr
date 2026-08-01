use super::*;
use crate::detect::hub::DetectHub;
use ffmpeg_bus::frame::RawFrameCmd;
use nvr_detect::{BBox, Detection, Detector};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// A hub with a `'static` lifetime, as `tap::run` requires. Leaked on purpose:
/// each test needs its own isolated hub, and the global `OnceLock` one can only
/// be installed once per process.
fn leaked_hub() -> &'static DetectHub {
    Box::leak(Box::new(DetectHub::new_for_test(
        vec![],
        std::path::PathBuf::from("."),
        500,
    )))
}

struct Fake(String);
impl Detector for Fake {
    fn name(&self) -> &str {
        &self.0
    }
    fn detect(&self, _rgb: &[u8], _w: u32, _h: u32) -> anyhow::Result<Vec<Detection>> {
        Ok(vec![Detection {
            class_id: 0,
            label: "person".into(),
            bbox: BBox {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0,
            },
            confidence: 0.9,
        }])
    }
}

#[tokio::test]
async fn fanout_runs_every_detector_concurrently() {
    let dets: Vec<Arc<dyn Detector>> = vec![Arc::new(Fake("a".into())), Arc::new(Fake("b".into()))];
    let rgb = Arc::new(vec![0u8; 3]);
    let results = fanout(&dets, rgb, 1, 1).await;

    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"a") && names.contains(&"b"));
    assert!(results.iter().all(|r| r.detections.len() == 1));
    assert!(results.iter().all(|r| r.error.is_none()));
}

#[tokio::test]
async fn tap_frees_its_registration_when_the_device_drops_the_stream() {
    // Device disconnect tears down the pipe, so the frame sender is dropped and
    // `recv` yields `Closed`. The tap ends without anyone calling `unregister`.
    let hub = leaked_hub();
    let (tx, rx) = tokio::sync::broadcast::channel(4);
    let cancel = CancellationToken::new();
    let epoch = hub.register("cam1", cancel.clone()).expect("register");

    drop(tx);
    run("cam1".to_string(), vec![], rx, 1000, hub, epoch, cancel).await;

    assert!(
        !hub.is_running("cam1"),
        "dead tap left a zombie registration"
    );
    // Which is what makes a restart on reconnect possible.
    assert!(hub.register("cam1", CancellationToken::new()).is_some());
}

#[tokio::test]
async fn tap_frees_its_registration_on_stream_eof() {
    let hub = leaked_hub();
    let (tx, rx) = tokio::sync::broadcast::channel(4);
    let cancel = CancellationToken::new();
    let epoch = hub.register("cam1", cancel.clone()).expect("register");

    assert!(tx.send(RawFrameCmd::EOF).is_ok());
    run("cam1".to_string(), vec![], rx, 1000, hub, epoch, cancel).await;

    assert!(
        !hub.is_running("cam1"),
        "dead tap left a zombie registration"
    );
}

#[tokio::test]
async fn cancelled_tap_does_not_evict_its_replacement() {
    // `unregister` already cleared the slot and a new tap took it over. The
    // cancelled tap's own cleanup must not tear that replacement down.
    let hub = leaked_hub();
    let (_tx, rx) = tokio::sync::broadcast::channel(4);
    let cancel = CancellationToken::new();
    let epoch = hub.register("cam1", cancel.clone()).expect("register");

    assert!(hub.unregister("cam1")); // fires `cancel`
    let replacement = CancellationToken::new();
    hub.register("cam1", replacement.clone())
        .expect("replacement registers");

    run("cam1".to_string(), vec![], rx, 1000, hub, epoch, cancel).await;

    assert!(hub.is_running("cam1"), "replacement tap was evicted");
    assert!(!replacement.is_cancelled());
}
