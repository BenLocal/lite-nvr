//! Process-global detection coordination: configured models, lazily-built
//! shared detectors, the registry of running per-pipe taps, and the latest
//! per-pipe result.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use nvr_detect::{Detector, DetectorConfig, UslsDetector};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use super::result::FrameResult;

static HUB: OnceLock<DetectHub> = OnceLock::new();

pub struct DetectHub {
    configs: Vec<DetectorConfig>,
    models_dir: PathBuf,
    sample_interval_ms: u64,
    detectors: AsyncMutex<Option<Vec<Arc<dyn Detector>>>>,
    running: Mutex<HashMap<String, CancellationToken>>,
    latest: Mutex<HashMap<String, FrameResult>>,
}

impl DetectHub {
    pub fn init(configs: Vec<DetectorConfig>, models_dir: PathBuf, sample_interval_ms: u64) {
        HUB.set(Self::new_for_test(configs, models_dir, sample_interval_ms))
            .ok()
            .expect("DetectHub::init called twice");
    }

    /// Construct a hub without installing it globally (for tests).
    pub fn new_for_test(
        configs: Vec<DetectorConfig>,
        models_dir: PathBuf,
        sample_interval_ms: u64,
    ) -> Self {
        Self {
            configs,
            models_dir,
            sample_interval_ms,
            detectors: AsyncMutex::new(None),
            running: Mutex::new(HashMap::new()),
            latest: Mutex::new(HashMap::new()),
        }
    }

    pub fn get() -> Option<&'static DetectHub> {
        HUB.get()
    }

    pub fn sample_interval_ms(&self) -> u64 {
        self.sample_interval_ms
    }

    pub fn config_names(&self) -> Vec<String> {
        self.configs.iter().map(|c| c.name.clone()).collect()
    }

    /// Build (or return cached) all configured detectors. Heavy on first call
    /// (loads every ONNX model); done on a blocking thread.
    pub async fn detectors(&self) -> anyhow::Result<Vec<Arc<dyn Detector>>> {
        let mut guard = self.detectors.lock().await;
        if let Some(d) = guard.as_ref() {
            return Ok(d.clone());
        }
        if self.configs.is_empty() {
            anyhow::bail!("no models configured (missing models.json in DETECT_MODELS_DIR?)");
        }
        let configs = self.configs.clone();
        let dir = self.models_dir.clone();
        // Build on a large-stack thread: ONNX Runtime session construction
        // overflows tokio's default blocking-thread stack. See
        // `super::spawn_big_stack`.
        let built = super::spawn_big_stack(
            "detect-build",
            move || -> anyhow::Result<Vec<Arc<dyn Detector>>> {
                let mut out: Vec<Arc<dyn Detector>> = Vec::new();
                for cfg in &configs {
                    let path = if std::path::Path::new(&cfg.model_file).is_absolute() {
                        PathBuf::from(&cfg.model_file)
                    } else {
                        dir.join(&cfg.model_file)
                    };
                    let det = UslsDetector::new(cfg, &path)?;
                    out.push(Arc::new(det));
                }
                Ok(out)
            },
        )
        .await
        .map_err(|_| anyhow::anyhow!("detector build thread died"))??;
        *guard = Some(built.clone());
        Ok(built)
    }

    /// Filter the shared detector list to a requested subset (by name). `None`
    /// or empty = all.
    pub fn detectors_named(
        &self,
        all: &[Arc<dyn Detector>],
        names: &Option<Vec<String>>,
    ) -> Vec<Arc<dyn Detector>> {
        match names {
            Some(want) if !want.is_empty() => all
                .iter()
                .filter(|d| want.iter().any(|n| n == d.name()))
                .cloned()
                .collect(),
            _ => all.to_vec(),
        }
    }

    pub fn register(&self, pipe: &str, cancel: CancellationToken) -> bool {
        let mut r = self.running.lock().unwrap();
        if r.contains_key(pipe) {
            return false;
        }
        r.insert(pipe.to_string(), cancel);
        true
    }

    pub fn unregister(&self, pipe: &str) -> bool {
        let mut r = self.running.lock().unwrap();
        if let Some(tok) = r.remove(pipe) {
            tok.cancel();
            true
        } else {
            false
        }
    }

    pub fn is_running(&self, pipe: &str) -> bool {
        self.running.lock().unwrap().contains_key(pipe)
    }

    pub fn store(&self, pipe: &str, result: FrameResult) {
        self.latest.lock().unwrap().insert(pipe.to_string(), result);
    }

    pub fn latest(&self, pipe: &str) -> Option<FrameResult> {
        self.latest.lock().unwrap().get(pipe).cloned()
    }
}
