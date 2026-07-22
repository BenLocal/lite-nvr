//! The one interface every backend implements.

use crate::types::Detection;

/// A single object-detection model. Implementations own their own pre/post-
/// processing; callers hand raw RGB8 bytes and get back unified `Detection`s in
/// original-frame pixel coordinates.
///
/// `detect` takes `&self` so a detector can be shared as `Arc<dyn Detector>`
/// and invoked concurrently across models; a backend needing interior
/// mutability (e.g. an ONNX session) hides it behind its own lock.
pub trait Detector: Send + Sync {
    fn name(&self) -> &str;
    fn detect(&self, rgb: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<Detection>>;
}
