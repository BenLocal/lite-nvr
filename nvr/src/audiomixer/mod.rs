//! Audio mixing console. Mixes several device audio streams into one or more
//! independently-published output buses, managed through the API. Wraps the
//! `nvr-audio-mixer` engine: resolves device ids to their ZLM streams, publishes
//! each bus to ZLM, and persists buses to KV so they restore at startup.

pub mod api;

use std::sync::LazyLock;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use nvr_audio_mixer::{AudioMixer, DEFAULT_VOLUME, MixerSnapshot};

/// KV key under which the live buses are persisted for restore.
const PERSIST_KEY: &str = "audio_mixer_buses";
/// Restore retry backoff bounds and total budget: retry `create_bus()` with
/// exponential backoff from MIN to MAX until BUDGET has elapsed, so a bus whose
/// device streams are slow to come up on a cold boot still restores.
const RESTORE_RETRY_MIN: Duration = Duration::from_secs(2);
const RESTORE_RETRY_MAX: Duration = Duration::from_secs(8);
const RESTORE_RETRY_BUDGET: Duration = Duration::from_secs(60);

pub(crate) static MIXER: LazyLock<AudioMixer> = LazyLock::new(AudioMixer::new);

/// Default publish URL for a bus when the caller doesn't give one.
fn default_publish_url(bus_id: &str) -> Result<String> {
    Ok(format!(
        "rtmp://127.0.0.1:{}/mixer/{bus_id}",
        crate::zlm::server::rtmp_port()?
    ))
}

/// The bus's FLV playback path on the dashboard's ZLM HTTP proxy.
pub fn bus_flv_url(bus_id: &str) -> String {
    format!("/media/mixer/{bus_id}.live.flv")
}

pub fn snapshot() -> MixerSnapshot {
    MIXER.snapshot()
}

/// Stop every mixer bus and source (their tasks are cancelled on Drop) for a
/// clean process shutdown. Does NOT clear the persisted buses — they restore on
/// the next start. Call before the process exits so no mixer/publish thread is
/// still writing into ZLM when its C runtime is torn down.
pub async fn shutdown() {
    output::shutdown().await;
    MIXER.shutdown().await;
}

pub mod native;
pub mod output;
pub mod sources;

/// Create a bus mixing the given `(device_id, volume)` inputs, publishing to ZLM.
pub async fn create_bus(
    bus_id: &str,
    publish_url: Option<String>,
    inputs: Vec<(String, u32)>,
) -> Result<()> {
    let bus_id = bus_id.trim();
    if bus_id.is_empty() {
        anyhow::bail!("bus id is required");
    }
    if inputs.is_empty() {
        anyhow::bail!("bus needs at least one input");
    }
    let cfg = output::config();
    anyhow::ensure!(
        !cfg.enabled || cfg.active_bus_id.as_deref() != Some(bus_id),
        "请先关闭或切换全局节目总线"
    );
    anyhow::ensure!(
        inputs.iter().all(|(_, volume)| *volume <= 400),
        "音量不能超过 400%"
    );
    sources::ensure(&inputs.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>()).await?;
    let publish = publish_url
        .filter(|u| !u.trim().is_empty())
        .map(Ok)
        .unwrap_or_else(|| default_publish_url(bus_id))?;
    MIXER
        .create_bus_with_profile(bus_id, &publish, &inputs, output::config().profile)
        .await?;
    persist_all().await;
    Ok(())
}

pub async fn remove_bus(bus_id: &str) -> Result<()> {
    let cfg = output::config();
    anyhow::ensure!(
        !cfg.enabled || cfg.active_bus_id.as_deref() != Some(bus_id),
        "请先关闭或切换全局节目总线"
    );
    if !MIXER.remove_bus(bus_id).await {
        anyhow::bail!("bus '{bus_id}' not found");
    }
    MIXER.prune_sources().await;
    persist_all().await;
    Ok(())
}

pub async fn add_input(bus_id: &str, source_id: &str, volume: u32) -> Result<()> {
    anyhow::ensure!(volume <= 400, "音量不能超过 400%");
    sources::ensure(&[source_id.to_string()]).await?;
    MIXER.bus_add_input(bus_id, source_id, volume)?;
    persist_all().await;
    Ok(())
}

pub async fn remove_input(bus_id: &str, source_id: &str) -> Result<()> {
    MIXER.bus_remove_input(bus_id, source_id)?;
    MIXER.prune_sources().await;
    persist_all().await;
    Ok(())
}

pub async fn set_volume(bus_id: &str, source_id: &str, volume: u32) -> Result<()> {
    anyhow::ensure!(volume <= 400, "音量不能超过 400%");
    MIXER.bus_set_volume(bus_id, source_id, volume)?;
    persist_all().await;
    Ok(())
}

pub async fn set_muted(bus_id: &str, source_id: &str, muted: bool) -> Result<()> {
    MIXER.bus_set_muted(bus_id, source_id, muted)?;
    persist_all().await;
    Ok(())
}

pub async fn set_channel_map(
    bus_id: &str,
    source_id: &str,
    mapping: nvr_audio_mixer::ChannelMap,
) -> Result<()> {
    MIXER.bus_set_channel_map(bus_id, source_id, mapping)?;
    persist_all().await;
    Ok(())
}

// ---- Persistence -----------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
struct PersistedInput {
    source_id: String,
    #[serde(default = "default_volume")]
    volume: u32,
    #[serde(default)]
    muted: bool,
    #[serde(default)]
    channel_map: nvr_audio_mixer::ChannelMap,
}

fn default_volume() -> u32 {
    DEFAULT_VOLUME
}

#[derive(Serialize, Deserialize, Clone)]
struct PersistedBus {
    id: String,
    publish_url: String,
    inputs: Vec<PersistedInput>,
}

/// Serialize every live bus to the KV store. Best-effort.
async fn persist_all() {
    let buses: Vec<PersistedBus> = MIXER
        .snapshot()
        .buses
        .into_iter()
        .map(|b| PersistedBus {
            id: b.id,
            publish_url: b.publish_url,
            inputs: b
                .inputs
                .into_iter()
                .map(|i| PersistedInput {
                    source_id: i.source_id,
                    volume: i.volume,
                    muted: i.muted,
                    channel_map: i.channel_map,
                })
                .collect(),
        })
        .collect();
    if let Err(e) = save_persisted(buses).await {
        log::warn!("audio mixer: failed to persist buses: {e:#}");
    }
}

async fn save_persisted(buses: Vec<PersistedBus>) -> Result<()> {
    let conn = crate::db::app_db_conn()?;
    nvr_db::config::set_json(PERSIST_KEY, &buses, &conn).await
}

async fn load_persisted() -> Result<Vec<PersistedBus>> {
    let conn = crate::db::app_db_conn()?;
    Ok(
        nvr_db::config::get_json::<Vec<PersistedBus>>(PERSIST_KEY, &conn)
            .await?
            .unwrap_or_default(),
    )
}

/// Rewrite a pre-rename persisted publish URL onto the current app so old saved
/// buses restore to the right stream: mixer output moved from the `live` app to
/// `mixer` (RTMP, port 8555). Port-anchored, a no-op for URLs already on `mixer`.
/// Inputs need no migration — their device pull URLs are rebuilt from
/// `device_audio_url` (the `device` app) on restore.
fn migrate_publish_url(url: &str) -> String {
    url.replace(":8555/live/", ":8555/mixer/")
}

/// Restore persisted buses at startup. Call AFTER device pipes have started so
/// the sources' ZLM streams exist (buses pull `rtsp://127.0.0.1:8554/device/{id}`).
/// Best-effort: a short grace period, then retry each bus's `create_bus()` with
/// backing-off attempts over a ~1-minute window so a bus whose device streams
/// are slow to come up on a cold boot still restores.
pub async fn restore_all() {
    let saved = match load_persisted().await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("audio mixer restore: load failed: {e:#}");
            return;
        }
    };
    if saved.is_empty() {
        return;
    }
    log::info!("audio mixer restore: {} bus(es) to restore", saved.len());

    for bus in saved {
        let id = bus.id.clone();
        let inputs: Vec<(String, u32)> = bus
            .inputs
            .iter()
            .map(|i| (i.source_id.clone(), i.volume))
            .collect();
        let publish = Some(migrate_publish_url(&bus.publish_url)).filter(|u| !u.trim().is_empty());
        let mut backoff = RESTORE_RETRY_MIN;
        let mut elapsed = Duration::ZERO;
        loop {
            let restored: Result<()> = async {
                for (source, _) in &inputs {
                    if let Some(address) = sources::capture_address(source) {
                        MIXER
                            .ensure_capture_source(source, &address, "alsa")
                            .await?;
                    } else {
                        MIXER.ensure_shared_source(source);
                    }
                }
                let target = publish
                    .clone()
                    .map(Ok)
                    .unwrap_or_else(|| default_publish_url(&id))?;
                MIXER
                    .create_bus_with_profile(&id, &target, &inputs, output::config().profile)
                    .await
            }
            .await;
            match restored {
                Ok(()) => {
                    for input in &bus.inputs {
                        let _ = MIXER.bus_set_channel_map(&id, &input.source_id, input.channel_map);
                        if input.muted {
                            let _ = MIXER.bus_set_muted(&id, &input.source_id, true);
                        }
                    }
                    log::info!("audio mixer restore: started '{id}'");
                    break;
                }
                Err(e) if elapsed < RESTORE_RETRY_BUDGET => {
                    log::warn!(
                        "audio mixer restore: '{id}' not ready yet ({e:#}); retry in {}s",
                        backoff.as_secs()
                    );
                    tokio::time::sleep(backoff).await;
                    elapsed += backoff;
                    backoff = (backoff * 2).min(RESTORE_RETRY_MAX);
                }
                Err(e) => {
                    log::error!(
                        "audio mixer restore: '{id}' gave up after ~{}s: {e:#}",
                        elapsed.as_secs()
                    );
                    break;
                }
            }
        }
    }
}
