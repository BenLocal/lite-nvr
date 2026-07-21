//! The per-frame, multi-model result served over the API.

use nvr_detect::ModelResult;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct FrameResult {
    /// Unix seconds when this frame was processed.
    pub ts: i64,
    pub frame_w: u32,
    pub frame_h: u32,
    pub models: Vec<ModelResult>,
}
