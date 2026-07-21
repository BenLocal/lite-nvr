//! Backend-agnostic object detection.

pub mod coco;
pub mod config;
pub mod detector;
pub mod set;
pub mod types;
pub mod usls_backend;

pub use config::DetectorConfig;
pub use detector::Detector;
pub use set::DetectorSet;
pub use types::{BBox, Detection, ModelResult};
pub use usls_backend::UslsDetector;
