use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use media_pipe_core::{Pipe, PipeConfig};
use tokio::{sync::RwLock, task::JoinHandle};
#[cfg(feature = "zlm")]
use tokio_util::sync::CancellationToken;

/// One managed background source per device id: either an ffmpeg-driven `Pipe`
/// (RTSP/file/v4l2 -> transcode -> ZLM) or a native worker thread (Xiaomi ->
/// ZLM) that bypasses ffmpeg. Keeping both in one registry lets device
/// add/update/remove and status work uniformly.
enum Entry {
    Pipe {
        pipe: Arc<Pipe>,
        handle: JoinHandle<()>,
    },
    #[cfg(feature = "zlm")]
    Worker {
        cancel: CancellationToken,
        handle: std::thread::JoinHandle<()>,
    },
}

impl Entry {
    /// Signal the source to stop (non-blocking).
    fn stop(&self) {
        match self {
            Entry::Pipe { pipe, .. } => pipe.cancel(),
            #[cfg(feature = "zlm")]
            Entry::Worker { cancel, .. } => cancel.cancel(),
        }
    }

    /// Wait for the source to fully unwind so its handles (input/output, ZLM
    /// Media) are released before a replacement with the same id starts.
    async fn join(self) {
        match self {
            Entry::Pipe { handle, .. } => {
                if let Err(e) = handle.await {
                    if !e.is_cancelled() {
                        log::warn!("pipe task ended with error: {}", e);
                    }
                }
            }
            #[cfg(feature = "zlm")]
            Entry::Worker { handle, .. } => {
                // The worker can be blocked in a socket read, so join it on a
                // blocking thread with a bound — a stalled camera must not hang
                // the manager. If it overruns we detach; it exits on its own
                // when the stream errors and drops the ZLM Media then.
                let join = tokio::task::spawn_blocking(move || {
                    let _ = handle.join();
                });
                if tokio::time::timeout(std::time::Duration::from_secs(3), join)
                    .await
                    .is_err()
                {
                    log::warn!("worker did not stop within 3s; detaching");
                }
            }
        }
    }

    fn is_started(&self) -> bool {
        match self {
            Entry::Pipe { pipe, .. } => pipe.is_started(),
            #[cfg(feature = "zlm")]
            Entry::Worker { .. } => true,
        }
    }
}

static PIPE_MANAGER: LazyLock<RwLock<HashMap<String, Entry>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Replace any existing entry for `id` with a freshly built one. The old entry
/// is cancelled and fully joined (outside the manager lock) BEFORE the new one
/// is built, so same-id handles (ZLM Media etc.) never overlap.
async fn upsert_entry(
    id: &str,
    build: impl FnOnce() -> Entry,
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

    // Phase 2: stop the old source and wait for it to unwind, outside the lock
    // so other readers aren't blocked.
    if let Some(old) = existing {
        old.stop();
        old.join().await;
    }

    // Phase 3: build (spawn) the new source and register it.
    let entry = build();
    let mut pipes = PIPE_MANAGER.write().await;
    pipes.insert(id.to_string(), entry);
    Ok(())
}

async fn upsert_pipe(id: &str, config: PipeConfig, update_if_exists: bool) -> anyhow::Result<()> {
    upsert_entry(
        id,
        move || {
            let pipe = Arc::new(Pipe::new(config));
            let pipe_for_task = Arc::clone(&pipe);
            let handle = tokio::spawn(async move {
                pipe_for_task.start().await;
            });
            Entry::Pipe { pipe, handle }
        },
        update_if_exists,
    )
    .await
}

pub(crate) async fn add_pipe(id: &str, config: PipeConfig) -> anyhow::Result<()> {
    upsert_pipe(id, config, false).await
}

pub(crate) async fn update_pipe(id: &str, config: PipeConfig) -> anyhow::Result<()> {
    upsert_pipe(id, config, true).await
}

/// Start (or replace) a native Xiaomi worker that pushes the camera stream into
/// `media`. Registered alongside pipes so the device lifecycle is uniform.
#[cfg(feature = "zlm")]
pub(crate) async fn upsert_xiaomi(
    id: &str,
    media: Arc<rszlm::media::Media>,
    cfg: crate::xiaomi::XiaomiConfig,
    update_if_exists: bool,
) -> anyhow::Result<()> {
    upsert_entry(
        id,
        move || {
            let cancel = CancellationToken::new();
            let handle = crate::xiaomi::spawn_to_zlm(cfg, media, cancel.clone());
            Entry::Worker { cancel, handle }
        },
        update_if_exists,
    )
    .await
}

pub(crate) async fn remove_pipe(id: &str) -> anyhow::Result<()> {
    let entry = {
        let mut pipes = PIPE_MANAGER.write().await;
        pipes.remove(id)
    };
    if let Some(entry) = entry {
        entry.stop();
        entry.join().await;
    }
    Ok(())
}

/// Running status for any entry: `Some(true/false)` for a pipe (false = not yet
/// started), `Some(true)` for a worker, `None` if absent.
pub(crate) async fn status(id: &str) -> Option<bool> {
    PIPE_MANAGER.read().await.get(id).map(|e| e.is_started())
}

pub(crate) async fn list_pipe_ids() -> Vec<String> {
    PIPE_MANAGER.read().await.keys().cloned().collect()
}
