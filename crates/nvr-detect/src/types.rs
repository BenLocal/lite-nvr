//! Unified detection output — identical shape regardless of which model or
//! backend produced it.

use serde::{Deserialize, Serialize};

/// Axis-aligned bounding box in original-frame pixel coordinates.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

/// One detected object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    pub class_id: usize,
    pub label: String,
    pub bbox: BBox,
    pub confidence: f32,
}

/// One model's result for a single frame, with its inference time. `error` is
/// set (and `detections` empty) if that model failed on this frame; other
/// models in the same batch are unaffected.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelResult {
    pub name: String,
    pub infer_ms: f64,
    pub detections: Vec<Detection>,
    pub error: Option<String>,
}

#[cfg(test)]
#[path = "types_test.rs"]
mod types_test;
