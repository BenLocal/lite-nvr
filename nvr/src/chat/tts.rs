//! The one TTS model shared by every chat session, loaded on first use.
//! Synthesis is serialized: one sentence at a time across all sessions.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Context;
use nvr_tts::{Audio, ModelKind, Tts, TtsConfig};
use tokio::sync::{Mutex, OnceCell};

/// The load result is kept even when it failed: a broken model directory is
/// reported to every turn instead of being reloaded each time.
static TTS: OnceCell<Result<Arc<Mutex<Tts>>, String>> = OnceCell::const_new();
static CLOSING: AtomicBool = AtomicBool::new(false);

/// The shared model, loading it on the first call.
pub(super) async fn get() -> Result<Arc<Mutex<Tts>>, String> {
    TTS.get_or_init(|| async {
        let configs = configs(
            &models_dir(),
            &std::env::var("NVR_TTS_RKNN_LIB").unwrap_or_else(|_| "librknnrt.so".into()),
        );
        let loaded = tokio::task::spawn_blocking(move || Tts::load_first(&configs))
            .await
            .context("TTS load task failed")
            .and_then(|r| r);
        match loaded {
            Ok(tts) => Ok(Arc::new(Mutex::new(tts))),
            Err(err) => {
                log::error!("chat: TTS unavailable: {err:#}");
                Err(format!("{err:#}"))
            }
        }
    })
    .await
    .clone()
}

/// `TTS_MODELS_DIR`, defaulting to the `scripts/download_tts_models.sh` layout.
fn models_dir() -> PathBuf {
    std::env::var("TTS_MODELS_DIR")
        .unwrap_or_else(|_| "third_party/tts-models".into())
        .into()
}

/// Backends in order of preference: the NPU MeloTTS, then the CPU one.
pub(super) fn configs(dir: &Path, rknn_lib: &str) -> Vec<TtsConfig> {
    let mut npu = TtsConfig::new(dir.join("MeloTTS-RKNN2"), ModelKind::MeloRknn);
    npu.rknn_lib = rknn_lib.to_string();
    let cpu = TtsConfig::new(dir.join("vits-melo-tts-zh_en"), ModelKind::Vits);
    vec![npu, cpu]
}

pub(super) async fn synthesize(tts: &Arc<Mutex<Tts>>, text: String) -> anyhow::Result<Audio> {
    let tts = tts.clone().lock_owned().await;
    anyhow::ensure!(!CLOSING.load(Ordering::Relaxed), "shutting down");
    tokio::task::spawn_blocking(move || tts.synthesize(&text)).await?
}

pub(super) async fn shutdown() {
    CLOSING.store(true, Ordering::Relaxed);
    if let Some(Ok(tts)) = TTS.get()
        && tokio::time::timeout(Duration::from_secs(2), tts.lock())
            .await
            .is_err()
    {
        log::warn!("chat: TTS still synthesizing after 2s; exiting anyway");
    }
}

#[cfg(test)]
#[path = "tts_test.rs"]
mod tts_test;
