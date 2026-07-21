//! Per-model configuration. A `models.json` manifest is a JSON array of these.

use serde::Deserialize;

fn default_input_size() -> u32 {
    640
}
fn default_conf() -> f32 {
    0.25
}
fn default_iou() -> f32 {
    0.45
}
fn default_device() -> String {
    "cpu".to_string()
}

/// One model's configuration. `model_file` is resolved relative to
/// `DETECT_MODELS_DIR` by the loader (see `nvr::detect::model_config`).
#[derive(Clone, Debug, Deserialize)]
pub struct DetectorConfig {
    /// Display name shown in results (e.g. "yolov8n").
    pub name: String,
    /// Path to the `.onnx` weights (relative to the models dir).
    pub model_file: String,
    /// Optional usls YOLO version hint (e.g. 8.0, 11.0). None = let usls infer.
    #[serde(default)]
    pub version: Option<f32>,
    /// Optional usls scale hint ("n"/"s"/"m"/"l"/"x").
    #[serde(default)]
    pub scale: Option<String>,
    /// Square model input size (pixels).
    #[serde(default = "default_input_size")]
    pub input_size: u32,
    /// Confidence threshold.
    #[serde(default = "default_conf")]
    pub conf: f32,
    /// IoU / NMS threshold.
    #[serde(default = "default_iou")]
    pub iou: f32,
    /// Class names in model-output order. Empty = default to COCO-80.
    #[serde(default)]
    pub class_names: Vec<String>,
    /// Inference device: "cpu" or e.g. "cuda:0".
    #[serde(default = "default_device")]
    pub device: String,
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
