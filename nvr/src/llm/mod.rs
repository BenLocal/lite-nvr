//! On-board LLM for Rockchip builds: loads an RKLLM model in a worker when
//! `NVR_LLM_MODEL` is set and serves an OpenAI-compatible API under
//! `/api/llm`. See `docs/voice-chat-spec.md`.

pub mod api;
mod config;
mod openai;

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use nvr_rkllm::Engine;

pub(crate) enum Llm {
    /// `NVR_LLM_MODEL` is not set.
    Disabled,
    /// The `NVR_LLM_*` settings are invalid.
    Invalid(String),
    Enabled {
        engine: Engine,
        /// Model id reported by the API (the model file's stem).
        model: String,
    },
}

static LLM: OnceLock<Llm> = OnceLock::new();

/// Read the config and start the isolated model worker when enabled.
/// Returns at once; the API reports `loading` until it is ready.
pub fn init() {
    let llm = match config::from_lookup(|name| std::env::var(name).ok()) {
        Ok(None) => {
            log::info!("llm: disabled (NVR_LLM_MODEL not set)");
            Llm::Disabled
        }
        Err(err) => {
            log::error!("llm: invalid configuration: {err:#}");
            Llm::Invalid(format!("{err:#}"))
        }
        Ok(Some(cfg)) => {
            let model = model_name(&cfg.model);
            let engine = Engine::spawn(cfg);
            let loading = engine.clone();
            let name = model.clone();
            tokio::spawn(async move {
                match loading.wait_ready().await {
                    Ok(()) => log::info!("llm: {name} ready"),
                    Err(err) => log::error!("llm: {name} unavailable: {err:#}"),
                }
            });
            Llm::Enabled { engine, model }
        }
    };
    if LLM.set(llm).is_err() {
        log::warn!("llm: init called twice; keeping the first configuration");
    }
}

pub(crate) fn get() -> &'static Llm {
    LLM.get_or_init(|| Llm::Disabled)
}

fn model_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rkllm".into())
}

/// Terminate and reap the isolated model worker before the host exits.
pub async fn shutdown() {
    if let Llm::Enabled { engine, .. } = get() {
        engine.shutdown(Duration::from_secs(2)).await;
    }
}
