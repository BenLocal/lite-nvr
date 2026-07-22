use super::*;
use nvr_detect::{BBox, Detection, Detector};
use std::sync::Arc;

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
