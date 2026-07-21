//! Real-time object detection for live pipes: taps decoded video, samples,
//! fans out to N models, and serves the latest per-frame comparison over REST.

pub mod convert;
pub mod hub;
pub mod result;

use std::path::PathBuf;

use nvr_detect::DetectorConfig;

/// Resolve the configured models from `DETECT_MODELS_DIR/models.json`. Returns
/// an empty config list (not an error) when the manifest is absent, so the app
/// still boots; `start` then reports "no models configured".
pub fn model_config() -> (Vec<DetectorConfig>, PathBuf) {
    let dir = std::env::var("DETECT_MODELS_DIR")
        .unwrap_or_else(|_| "third_party/detect-models".to_string());
    let dir = PathBuf::from(dir);
    let manifest = dir.join("models.json");
    let configs = match std::fs::read_to_string(&manifest) {
        Ok(s) => serde_json::from_str::<Vec<DetectorConfig>>(&s).unwrap_or_else(|e| {
            log::warn!("detect: bad {}: {e:#}", manifest.display());
            vec![]
        }),
        Err(_) => {
            log::info!("detect: no manifest at {}", manifest.display());
            vec![]
        }
    };
    (configs, dir)
}

#[cfg(test)]
#[path = "hub_test.rs"]
mod hub_test;
