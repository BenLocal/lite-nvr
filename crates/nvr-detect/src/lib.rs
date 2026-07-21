//! Backend-agnostic object detection.

pub mod coco;
pub mod config;
pub mod types;

pub use config::DetectorConfig;
pub use types::{BBox, Detection, ModelResult};
