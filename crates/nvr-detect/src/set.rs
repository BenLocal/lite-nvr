//! A synchronous fan-out over several detectors: run one image through all of
//! them and collect each model's timed result. Used by the offline
//! `detect-compare` example and tests; the real-time path (nvr) runs the same
//! detectors concurrently.

use std::time::Instant;

use crate::detector::Detector;
use crate::types::ModelResult;

pub struct DetectorSet {
    detectors: Vec<Box<dyn Detector>>,
}

impl DetectorSet {
    pub fn new(detectors: Vec<Box<dyn Detector>>) -> Self {
        Self { detectors }
    }

    pub fn len(&self) -> usize {
        self.detectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.detectors.is_empty()
    }

    /// Run every detector on the same image, in order. Each model is timed; a
    /// failing model yields a `ModelResult` with empty detections and `error`
    /// set, leaving the others intact.
    pub fn detect_all(&self, rgb: &[u8], width: u32, height: u32) -> Vec<ModelResult> {
        self.detectors
            .iter()
            .map(|d| {
                let start = Instant::now();
                let res = d.detect(rgb, width, height);
                let infer_ms = start.elapsed().as_secs_f64() * 1000.0;
                match res {
                    Ok(detections) => ModelResult {
                        name: d.name().to_string(),
                        infer_ms,
                        detections,
                        error: None,
                    },
                    Err(e) => ModelResult {
                        name: d.name().to_string(),
                        infer_ms,
                        detections: vec![],
                        error: Some(format!("{e:#}")),
                    },
                }
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "set_test.rs"]
mod set_test;
