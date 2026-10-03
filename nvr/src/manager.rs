use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use crate::detect::hub::DetectHub;
use media_pipe_core::{InputConfig, Pipe, PipeConfig};
use nvr_db::device::DeviceInfo;
use tokio::{sync::RwLock, task::JoinHandle};
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
    // This lock only swaps/clones Arc values; no user code runs under it.
    ReconnectingPipe {
        pipe: Arc<std::sync::RwLock<Option<Arc<Pipe>>>>,
        cancel: CancellationToken,
        handle: JoinHandle<()>,
    },
    Worker {
        cancel: CancellationToken,
        handle: std::thread::JoinHandle<()>,
    },
    /// An async supervisor task (e.g. a platform live-stream worker that
    /// re-resolves its pull URL and re-runs an inner pipe on every reconnect).
    Task {
        cancel: CancellationToken,
        handle: JoinHandle<()>,
    },
}

impl Entry {
    /// Signal the source to stop (non-blocking).
    fn stop(&self) {
        match self {
            Entry::Pipe { pipe, .. } => pipe.cancel(),
            Entry::ReconnectingPipe { pipe, cancel, .. } => {
                cancel.cancel();
                if let Some(pipe) = pipe.read().expect("network pipe lock poisoned").as_ref() {
                    pipe.cancel();
                }
            }
            Entry::Worker { cancel, .. } => cancel.cancel(),
            Entry::Task { cancel, .. } => cancel.cancel(),
        }
    }

    /// Wait for the source to fully unwind so its handles (input/output, ZLM
    /// Media) are released before a replacement with the same id starts.
    async fn join(self) {
        match self {
            Entry::Pipe { handle, .. } | Entry::ReconnectingPipe { handle, .. } => {
                if let Err(e) = handle.await {
                    if !e.is_cancelled() {
                        log::warn!("pipe task ended with error: {}", e);
                    }
                }
            }
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
            Entry::Task { handle, .. } => {
                if let Err(e) = handle.await {
                    if !e.is_cancelled() {
                        log::warn!("stream worker task ended with error: {}", e);
                    }
                }
            }
        }
    }

    fn is_started(&self) -> bool {
        match self {
            Entry::Pipe { pipe, .. } => pipe.is_started(),
            Entry::ReconnectingPipe { pipe, .. } => pipe
                .read()
                .expect("network pipe lock poisoned")
                .as_ref()
                .is_some_and(|pipe| pipe.is_started()),
            Entry::Worker { .. } | Entry::Task { .. } => true,
        }
    }
}

static PIPE_MANAGER: LazyLock<RwLock<HashMap<String, Entry>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

static MUTATIONS: LazyLock<crate::lifecycle::KeyedLocks> = LazyLock::new(Default::default);
// Operations take a read lease; shutdown takes the write lease and closes the
// manager, so an operation cannot insert a source after the shutdown drain.
static SHUTTING_DOWN: RwLock<bool> = RwLock::const_new(false);

/// Replace any existing entry for `id` with a freshly built one. The old entry
/// is cancelled and fully joined (outside the manager lock) BEFORE the new one
/// is built, so same-id handles (ZLM Media etc.) never overlap.
async fn upsert_entry(
    id: &str,
    build: impl FnOnce() -> Entry,
    update_if_exists: bool,
) -> anyhow::Result<()> {
    let lifecycle = SHUTTING_DOWN.read().await;
    if *lifecycle {
        anyhow::bail!("media manager is shutting down");
    }
    let _operation = MUTATIONS.lock(id).await;
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

/// RTSP over UDP (FFmpeg's default) drops packets on lossy/jittery links, which
/// corrupts the H264 stream ("RTP: missed packets" -> decode errors). Force TCP
/// transport with a socket timeout for rtsp:// inputs. Transport policy lives
/// here (the app) so `media-pipe-core` stays input-agnostic.
fn input_options(input: &InputConfig) -> Option<HashMap<String, String>> {
    match input {
        InputConfig::Network { url } if url.starts_with("rtsp://") => Some(HashMap::from([
            ("rtsp_transport".to_string(), "tcp".to_string()),
            ("stimeout".to_string(), "5000000".to_string()),
        ])),
        _ => None,
    }
}

async fn upsert_pipe(id: &str, config: PipeConfig, update_if_exists: bool) -> anyhow::Result<()> {
    upsert_entry(
        id,
        move || {
            let options = input_options(&config.input);
            let pipe = Arc::new(Pipe::new(config));
            let pipe_for_task = Arc::clone(&pipe);
            let handle = tokio::spawn(async move {
                pipe_for_task.start(options).await;
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

/// Keep an ordinary network camera alive across EOF/read/open failures. Each
/// session builds fresh ZLM tracks/coordinators, and exposes its current pipe
/// for ASR/detection subscriptions just like a non-supervised pipe.
pub(crate) async fn upsert_network_device(
    device: &DeviceInfo,
    detect_hub: &'static DetectHub,
) -> anyhow::Result<()> {
    let device = device.clone();
    let id = device.id.clone();
    upsert_entry(
        &id,
        move || {
            let cancel = CancellationToken::new();
            let current = Arc::new(std::sync::RwLock::new(None));
            let handle = spawn_network_device(device, detect_hub, current.clone(), cancel.clone());
            Entry::ReconnectingPipe {
                pipe: current,
                cancel,
                handle,
            }
        },
        true,
    )
    .await
}

fn spawn_network_device(
    device: DeviceInfo,
    detect_hub: &'static DetectHub,
    current: Arc<std::sync::RwLock<Option<Arc<Pipe>>>>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = std::time::Duration::from_secs(2);
        while !cancel.is_cancelled() {
            let started = std::time::Instant::now();
            let media = Arc::new(rszlm::media::Media::new_with_default_vhost(
                crate::init::device::DEVICE_APP,
                &device.id,
                0.0,
                device.record,
                false,
            ));
            let config = PipeConfig {
                input: InputConfig::Network {
                    url: device.input_value.clone(),
                },
                outputs: media_pipe_zlm::zlm_outputs(media, device.include_audio),
            };
            let options = input_options(&config.input);
            let pipe = Arc::new(Pipe::new(config));
            *current.write().expect("network pipe lock poisoned") = Some(pipe.clone());
            let running = pipe.clone();
            let mut session = tokio::spawn(async move {
                running.start(options).await;
            });
            crate::detect::control::reconcile_detection(detect_hub, &device).await;
            tokio::select! {
                _ = cancel.cancelled() => {
                    pipe.cancel();
                    let _ = (&mut session).await;
                }
                result = &mut session => {
                    if let Err(e) = result { log::warn!("network device {} task failed: {e}", device.id); }
                }
            }
            *current.write().expect("network pipe lock poisoned") = None;
            detect_hub.stop(&device.id);
            drop(pipe);
            if cancel.is_cancelled() {
                break;
            }
            if started.elapsed() >= std::time::Duration::from_secs(30) {
                backoff = std::time::Duration::from_secs(2);
            }
            log::warn!(
                "network device {}: session ended, retry in {backoff:?}",
                device.id
            );
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(std::time::Duration::from_secs(60));
        }
    })
}

/// Start (or replace) a native Xiaomi worker that pushes the camera stream into
/// `media`. Registered alongside pipes so the device lifecycle is uniform.
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

/// Start (or replace) a platform live-stream worker: it resolves the room/page
/// URL via yt-dlp and (re)runs an inner pipe into `media`, re-resolving on
/// every reconnect because the pull addresses are signed and expire.
pub(crate) async fn upsert_stream(
    id: &str,
    media: Arc<rszlm::media::Media>,
    page_url: String,
    include_audio: bool,
    update_if_exists: bool,
) -> anyhow::Result<()> {
    let device_id = id.to_string();
    upsert_entry(
        id,
        move || {
            let cancel = CancellationToken::new();
            let handle = crate::livestream::spawn_stream_device(
                device_id,
                page_url,
                media,
                include_audio,
                cancel.clone(),
            );
            Entry::Task { cancel, handle }
        },
        update_if_exists,
    )
    .await
}

/// Start (or replace) an ONVIF ingestion worker: it resolves the camera's RTSP
/// stream URI over ONVIF and (re)runs the same inner pipe into `media`,
/// re-resolving on every reconnect because a camera reboot or config change can
/// move the URI. Symmetric with [`upsert_stream`], differing only in the
/// resolve step.
pub(crate) async fn upsert_onvif(
    id: &str,
    media: Arc<rszlm::media::Media>,
    cfg: nvr_onvif::OnvifConfig,
    include_audio: bool,
    update_if_exists: bool,
) -> anyhow::Result<()> {
    let device_id = id.to_string();
    upsert_entry(
        id,
        move || {
            let cancel = CancellationToken::new();
            let handle = crate::onvif::ingest::spawn_onvif_device(
                device_id,
                cfg,
                media,
                include_audio,
                cancel.clone(),
            );
            Entry::Task { cancel, handle }
        },
        update_if_exists,
    )
    .await
}

pub(crate) async fn remove_pipe(id: &str) -> anyhow::Result<()> {
    let _lifecycle = SHUTTING_DOWN.read().await;
    let _operation = MUTATIONS.lock(id).await;
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

/// Stop and join every managed pipe/worker for a clean process shutdown, so no
/// pipe thread is still pushing into a ZLM `Media` when the process tears down
/// its C runtime.
pub(crate) async fn shutdown() {
    let mut lifecycle = SHUTTING_DOWN.write().await;
    *lifecycle = true;
    let entries: Vec<Entry> = { PIPE_MANAGER.write().await.drain().map(|(_, e)| e).collect() };
    for e in &entries {
        e.stop();
    }
    for e in entries {
        e.join().await;
    }
}

/// Running status for any entry: `Some(true/false)` for a pipe (false = not yet
/// started), `Some(true)` for a worker, `None` if absent.
pub(crate) async fn status(id: &str) -> Option<bool> {
    PIPE_MANAGER.read().await.get(id).map(|e| e.is_started())
}

pub(crate) async fn list_pipe_ids() -> Vec<String> {
    PIPE_MANAGER.read().await.keys().cloned().collect()
}

/// Fetch a shared handle to the `Pipe` for `id`, if one is registered. Native
/// worker entries have no `Pipe`, so they return `None`.
pub(crate) async fn get_pipe(id: &str) -> Option<Arc<Pipe>> {
    PIPE_MANAGER.read().await.get(id).and_then(|e| match e {
        Entry::Pipe { pipe, .. } => Some(pipe.clone()),
        Entry::ReconnectingPipe { pipe, .. } => {
            pipe.read().expect("network pipe lock poisoned").clone()
        }
        Entry::Worker { .. } | Entry::Task { .. } => None,
    })
}

#[cfg(test)]
#[path = "manager_test.rs"]
mod manager_test;
