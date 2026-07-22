//! Real-time object detection for live pipes: taps decoded video, samples,
//! fans out to N models, and serves the latest per-frame comparison over REST.

pub mod api;
pub mod convert;
pub mod hub;
pub mod result;
pub mod tap;

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

/// Spawn a blocking closure on a dedicated thread with a large (16 MiB) stack,
/// returning a receiver for its result. The thread starts immediately, so
/// several calls run concurrently; `await` the receiver for the value (an `Err`
/// means the thread died without producing one, e.g. it panicked).
///
/// ONNX Runtime session construction (and inference) recurse deeply enough to
/// overflow tokio's default ~2 MiB `spawn_blocking` thread stack — building a
/// real YOLO model there aborts the process with a stack overflow. The 8 MiB
/// main-thread stack is enough (the offline example proves it), so we give the
/// ONNX work its own thread with generous headroom.
pub(crate) fn spawn_big_stack<T, F>(name: &'static str, f: F) -> tokio::sync::oneshot::Receiver<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let _ = tx.send(f());
        })
        .expect("spawn detect ONNX thread");
    rx
}

#[cfg(test)]
#[path = "hub_test.rs"]
mod hub_test;
