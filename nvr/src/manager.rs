use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use tokio::{sync::RwLock, task::JoinHandle};

use crate::media::{pipe::Pipe, types::PipeConfig};

struct PipeEntry {
    pipe: Arc<Pipe>,
    handle: JoinHandle<()>,
}

static PIPE_MANAGER: LazyLock<RwLock<HashMap<String, PipeEntry>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

async fn upsert_pipe(
    id: &str,
    config: PipeConfig,
    update_if_exists: bool,
) -> anyhow::Result<()> {
    // Phase 1: take ownership of any existing entry under the write lock.
    let existing = {
        let mut pipes = PIPE_MANAGER.write().await;
        if pipes.contains_key(id) && !update_if_exists {
            return Err(anyhow::anyhow!("Pipe already exists"));
        }
        pipes.remove(id)
    };

    // Phase 2: cancel the old pipe and wait for its task to fully unwind
    // (releases input/output handles, ZLM Media, etc.) BEFORE starting the new one.
    // Done outside the manager lock so other readers aren't blocked.
    if let Some(old) = existing {
        old.pipe.cancel();
        if let Err(e) = old.handle.await {
            if !e.is_cancelled() {
                log::warn!("Pipe '{}' previous task ended with error: {}", id, e);
            }
        }
    }

    // Phase 3: spawn the new pipe and register it.
    let pipe = Arc::new(Pipe::new(config));
    let pipe_for_task = Arc::clone(&pipe);
    let handle = tokio::spawn(async move {
        pipe_for_task.start().await;
    });

    let mut pipes = PIPE_MANAGER.write().await;
    pipes.insert(id.to_string(), PipeEntry { pipe, handle });
    Ok(())
}

pub(crate) async fn add_pipe(id: &str, config: PipeConfig) -> anyhow::Result<()> {
    upsert_pipe(id, config, false).await
}

pub(crate) async fn update_pipe(id: &str, config: PipeConfig) -> anyhow::Result<()> {
    upsert_pipe(id, config, true).await
}

pub(crate) async fn remove_pipe(id: &str) -> anyhow::Result<()> {
    let entry = {
        let mut pipes = PIPE_MANAGER.write().await;
        pipes.remove(id)
    };
    if let Some(entry) = entry {
        entry.pipe.cancel();
        if let Err(e) = entry.handle.await {
            if !e.is_cancelled() {
                log::warn!("Pipe '{}' task ended with error during removal: {}", id, e);
            }
        }
    }
    Ok(())
}

pub(crate) async fn get_pipe(id: &str) -> Option<Arc<Pipe>> {
    PIPE_MANAGER
        .read()
        .await
        .get(id)
        .map(|entry| Arc::clone(&entry.pipe))
}

pub(crate) async fn list_pipe_ids() -> Vec<String> {
    PIPE_MANAGER.read().await.keys().cloned().collect()
}
