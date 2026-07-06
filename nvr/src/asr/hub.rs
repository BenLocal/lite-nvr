//! Process-global ASR coordination: the Socket.IO handle, the shared models
//! (lazy), and the registry of running per-pipe tap tasks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use nvr_asr::{AsrConfig, AsrModels};
use socketioxide::SocketIo;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;

static HUB: OnceLock<AsrHub> = OnceLock::new();

pub struct AsrHub {
    io: SocketIo,
    config: AsrConfig,
    models: AsyncMutex<Option<Arc<AsrModels>>>,
    running: Mutex<HashMap<String, CancellationToken>>,
}

impl AsrHub {
    /// Install the global hub once at startup. Panics if called twice.
    pub fn init(io: SocketIo, config: AsrConfig) {
        HUB.set(AsrHub {
            io,
            config,
            models: AsyncMutex::new(None),
            running: Mutex::new(HashMap::new()),
        })
        .ok()
        .expect("AsrHub::init called twice");
    }

    pub fn get() -> Option<&'static AsrHub> {
        HUB.get()
    }

    pub fn io(&self) -> &SocketIo {
        &self.io
    }

    /// Load (or return cached) shared models. Heavy on first call.
    pub async fn models(&self) -> anyhow::Result<Arc<AsrModels>> {
        let mut guard = self.models.lock().await;
        if let Some(m) = guard.as_ref() {
            return Ok(m.clone());
        }
        let m = AsrModels::load(self.config.clone())?;
        *guard = Some(m.clone());
        Ok(m)
    }

    /// Register a running tap; returns false if already running.
    pub fn register(&self, pipe: &str, cancel: CancellationToken) -> bool {
        let mut r = self.running.lock().unwrap();
        if r.contains_key(pipe) {
            return false;
        }
        r.insert(pipe.to_string(), cancel);
        true
    }

    /// Cancel + deregister a running tap. Returns true if one was running.
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
}
