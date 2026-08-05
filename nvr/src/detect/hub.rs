//! Process-global detection coordination: configured models, lazily-built
//! shared detectors, the registry of running per-pipe taps, and the latest
//! per-pipe result.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use nvr_detect::{Detector, DetectorConfig, UslsDetector};
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

use super::result::FrameResult;

static HUB: OnceLock<DetectHub> = OnceLock::new();

/// Identifies one generation of a pipe's tap. A tap that ends on its own clears
/// its slot with [`DetectHub::unregister_tap`], which matches only while that
/// same generation is still registered — so a tap still unwinding after being
/// cancelled can never evict the tap that replaced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TapEpoch(u64);

pub struct DetectHub {
    configs: Vec<DetectorConfig>,
    models_dir: PathBuf,
    sample_interval_ms: u64,
    detectors: AsyncMutex<Option<Vec<Arc<dyn Detector>>>>,
    running: Mutex<HashMap<String, (TapEpoch, CancellationToken)>>,
    auto_start: Mutex<HashMap<String, (u64, CancellationToken)>>,
    next_auto_start: AtomicU64,
    next_epoch: AtomicU64,
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
            auto_start: Mutex::new(HashMap::new()),
            next_auto_start: AtomicU64::new(0),
            next_epoch: AtomicU64::new(0),
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

    /// Start a new device auto-start generation, cancelling any older retry
    /// task for the same device.
    pub fn begin_auto_start(&self, pipe: &str) -> (u64, CancellationToken) {
        let token = CancellationToken::new();
        let generation = self.next_auto_start.fetch_add(1, Ordering::Relaxed);
        let mut pending = self.auto_start.lock().unwrap();
        if let Some((_, old)) = pending.insert(pipe.to_string(), (generation, token.clone())) {
            old.cancel();
        }
        (generation, token)
    }

    /// Cancel a pending auto-start and prevent it from claiming a tap later.
    pub fn cancel_auto_start(&self, pipe: &str) {
        if let Some((_, token)) = self.auto_start.lock().unwrap().remove(pipe) {
            token.cancel();
        }
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

    /// Claim `pipe` for a new tap. Returns the generation to hand to
    /// `tap::run`, or `None` if a tap is already registered.
    pub fn register(&self, pipe: &str, cancel: CancellationToken) -> Option<TapEpoch> {
        let mut r = self.running.lock().unwrap();
        if r.contains_key(pipe) {
            return None;
        }
        let epoch = TapEpoch(self.next_epoch.fetch_add(1, Ordering::Relaxed));
        r.insert(pipe.to_string(), (epoch, cancel));
        Some(epoch)
    }

    /// Claim a tap only if the auto-start generation is still current. The
    /// pending-generation lock is held through registration so cancellation
    /// cannot happen between the final check and the running-slot insert.
    pub fn register_auto_start(
        &self,
        pipe: &str,
        generation: u64,
        cancel: CancellationToken,
    ) -> Option<TapEpoch> {
        let mut pending = self.auto_start.lock().unwrap();
        if pending
            .get(pipe)
            .is_none_or(|(current, _)| *current != generation)
        {
            return None;
        }
        let mut running = self.running.lock().unwrap();
        if running.contains_key(pipe) {
            return None;
        }
        let epoch = TapEpoch(self.next_epoch.fetch_add(1, Ordering::Relaxed));
        running.insert(pipe.to_string(), (epoch, cancel));
        pending.remove(pipe);
        Some(epoch)
    }

    /// Stop whichever tap holds `pipe`, whatever its generation.
    pub fn unregister(&self, pipe: &str) -> bool {
        let mut r = self.running.lock().unwrap();
        if let Some((_, tok)) = r.remove(pipe) {
            tok.cancel();
            true
        } else {
            false
        }
    }

    /// Release `pipe` on behalf of a tap that has ended by itself, but only if
    /// `epoch` is still the registered generation.
    ///
    /// A tap can end without anyone calling [`Self::unregister`] — stream EOF,
    /// or the frame sender dropping when a device disconnects. Without this the
    /// slot would outlive the task, leaving `is_running` permanently true and
    /// blocking any restart on reconnect.
    pub fn unregister_tap(&self, pipe: &str, epoch: TapEpoch) -> bool {
        let mut r = self.running.lock().unwrap();
        match r.get(pipe) {
            Some((current, _)) if *current == epoch => {
                r.remove(pipe);
                true
            }
            _ => false,
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
