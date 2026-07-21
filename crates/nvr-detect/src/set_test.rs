use super::*;
use crate::types::{BBox, Detection};

struct FakeDetector {
    name: String,
    out: anyhow::Result<Vec<Detection>>,
}

impl Detector for FakeDetector {
    fn name(&self) -> &str {
        &self.name
    }
    fn detect(&self, _rgb: &[u8], _w: u32, _h: u32) -> anyhow::Result<Vec<Detection>> {
        match &self.out {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(anyhow::anyhow!("{e:#}")),
        }
    }
}

fn det(label: &str) -> Detection {
    Detection {
        class_id: 0,
        label: label.to_string(),
        bbox: BBox {
            x1: 0.0,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        },
        confidence: 0.9,
    }
}

#[test]
fn detect_all_preserves_order_times_each_and_captures_errors() {
    let set = DetectorSet::new(vec![
        Box::new(FakeDetector {
            name: "a".into(),
            out: Ok(vec![det("person")]),
        }),
        Box::new(FakeDetector {
            name: "b".into(),
            out: Err(anyhow::anyhow!("boom")),
        }),
    ]);
    let rgb = vec![0u8; 3]; // 1x1 RGB
    let results = set.detect_all(&rgb, 1, 1);

    assert_eq!(results.len(), 2);
    // Order preserved.
    assert_eq!(results[0].name, "a");
    assert_eq!(results[1].name, "b");
    // Success path.
    assert_eq!(results[0].detections.len(), 1);
    assert_eq!(results[0].detections[0].label, "person");
    assert!(results[0].error.is_none());
    // Error path: empty detections, error populated, other models unaffected.
    assert!(results[1].detections.is_empty());
    assert!(results[1].error.as_deref().unwrap().contains("boom"));
    // Every model is timed (>= 0.0).
    assert!(results[0].infer_ms >= 0.0);
    assert!(results[1].infer_ms >= 0.0);
}
