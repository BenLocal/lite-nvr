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
/// ZLM RTSP pull endpoint — device streams are published here as app `live`.
const ZLM_RTSP: &str = "rtsp://127.0.0.1:8554";
/// ZLM RTMP publish endpoint (see `zlm::server`: `rtmp_server_start(8555)`).
const ZLM_RTMP: &str = "rtmp://127.0.0.1:8555";

static MIXER: LazyLock<AudioMixer> = LazyLock::new(AudioMixer::new);

/// A device's audio stream URL on ZLM (source id == device id).
fn device_audio_url(source_id: &str) -> String {
    format!("{ZLM_RTSP}/live/{source_id}")
}

/// Default publish URL for a bus when the caller doesn't give one.
fn default_publish_url(bus_id: &str) -> String {
    format!("{ZLM_RTMP}/live/{bus_id}")
}

/// The bus's FLV playback path on the dashboard's ZLM HTTP proxy.
pub fn bus_flv_url(bus_id: &str) -> String {
    format!("/media/live/{bus_id}.live.flv")
}

pub fn snapshot() -> MixerSnapshot {
    MIXER.snapshot()
}

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
    for (source_id, _) in &inputs {
        MIXER
            .ensure_source(source_id, &device_audio_url(source_id))
            .await?;
    }
    let publish = publish_url
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| default_publish_url(bus_id));
    MIXER.create_bus(bus_id, &publish, &inputs).await?;
    persist_all().await;
    Ok(())
}

pub async fn remove_bus(bus_id: &str) -> Result<()> {
    if !MIXER.remove_bus(bus_id) {
        anyhow::bail!("bus '{bus_id}' not found");
    }
    persist_all().await;
    Ok(())
}

pub async fn add_input(bus_id: &str, source_id: &str, volume: u32) -> Result<()> {
    MIXER
        .ensure_source(source_id, &device_audio_url(source_id))
        .await?;
    MIXER.bus_add_input(bus_id, source_id, volume)?;
    persist_all().await;
    Ok(())
}

pub async fn remove_input(bus_id: &str, source_id: &str) -> Result<()> {
    MIXER.bus_remove_input(bus_id, source_id)?;
    persist_all().await;
    Ok(())
}

pub async fn set_volume(bus_id: &str, source_id: &str, volume: u32) -> Result<()> {
    MIXER.bus_set_volume(bus_id, source_id, volume)?;
    persist_all().await;
    Ok(())
}

pub async fn set_muted(bus_id: &str, source_id: &str, muted: bool) -> Result<()> {
    MIXER.bus_set_muted(bus_id, source_id, muted)?;
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

/// Restore persisted buses at startup. Call AFTER device pipes have started so
/// the sources' ZLM streams exist (buses pull `rtsp://127.0.0.1:8554/live/{id}`).
/// Best-effort, with a short grace period and a few retries.
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
    tokio::time::sleep(Duration::from_secs(3)).await;

    for bus in saved {
        let id = bus.id.clone();
        let inputs: Vec<(String, u32)> = bus
            .inputs
            .iter()
            .map(|i| (i.source_id.clone(), i.volume))
            .collect();
        let publish = Some(bus.publish_url.clone()).filter(|u| !u.trim().is_empty());
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match create_bus(&id, publish.clone(), inputs.clone()).await {
                Ok(()) => {
                    for input in &bus.inputs {
                        if input.muted {
                            let _ = set_muted(&id, &input.source_id, true).await;
                        }
                    }
                    log::info!("audio mixer restore: started '{id}'");
                    break;
                }
                Err(e) if attempt < 3 => {
                    log::warn!(
                        "audio mixer restore: '{id}' attempt {attempt} failed ({e:#}); retrying"
                    );
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(e) => {
                    log::error!("audio mixer restore: '{id}' gave up after {attempt}: {e:#}");
                    break;
                }
            }
        }
    }
}
