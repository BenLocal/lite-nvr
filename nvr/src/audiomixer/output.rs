//! Process-wide program selection and original-input subscriptions.
use media_pipe_core::Pipe;
use nvr_audio_mixer::{AudioProfile, MixMode};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex, RwLock, Weak},
};
use tokio_util::sync::CancellationToken;

const CONFIG_KEY: &str = "audio_mixer_output";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProgramConfig {
    pub enabled: bool,
    pub mode: MixMode,
    pub active_bus_id: Option<String>,
    pub profile: AudioProfile,
    pub version: u64,
}

#[derive(Serialize)]
pub struct OutputState {
    pub device_id: String,
    pub use_global_audio: bool,
    pub effective_version: Option<u64>,
    pub state: &'static str,
}

#[derive(Serialize)]
pub struct ProgramState {
    pub config: ProgramConfig,
    pub outputs: Vec<OutputState>,
    pub last_error: Option<String>,
}

static CONFIG: LazyLock<RwLock<ProgramConfig>> =
    LazyLock::new(|| RwLock::new(ProgramConfig::default()));
static PIPES: LazyLock<Mutex<HashMap<String, Weak<Pipe>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static MONITOR_CANCEL: LazyLock<CancellationToken> = LazyLock::new(CancellationToken::new);
static MONITOR: Mutex<Option<tokio::task::JoinHandle<()>>> = Mutex::new(None);
static UPDATES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn config() -> ProgramConfig {
    CONFIG.read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn register_pipe(id: &str, pipe: &Arc<Pipe>) {
    PIPES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id.to_string(), Arc::downgrade(pipe));
}

pub async fn audio_for_output(
    id: &str,
    include_original: bool,
    settings: Option<&nvr_db::device::OutputConfig>,
) -> anyhow::Result<Option<ffmpeg_bus::shared_audio::SharedEncodedAudio>> {
    if let Some(bus) = settings.and_then(|s| s.audio_bus_id.as_deref()) {
        if !super::bus_enabled(bus) {
            return Ok(None);
        }
        let mode = match settings.and_then(|s| s.audio_mode).unwrap_or_default() {
            nvr_db::device::AudioMixMode::Program => MixMode::Program,
            nvr_db::device::AudioMixMode::AddOriginal => MixMode::AddOriginal,
        };
        return Ok(Some(
            super::MIXER
                .output_audio(bus, id, mode, include_original)
                .await?,
        ));
    }
    let use_global = settings.and_then(|s| s.use_global_audio).unwrap_or(true);
    let cfg = config();
    if !cfg.enabled || !use_global {
        return Ok(None);
    }
    let bus = cfg
        .active_bus_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("未选择节目总线"))?;
    Ok(Some(
        super::MIXER
            .output_audio(bus, id, cfg.mode, include_original)
            .await?
            .with_revision(cfg.version),
    ))
}

pub async fn state() -> anyhow::Result<ProgramState> {
    let cfg = config();
    let conn = crate::db::app_db_conn()?;
    let devices = nvr_db::device::list(&conn).await?;
    let pipes: HashMap<_, _> = PIPES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter_map(|(id, p)| p.upgrade().map(|p| (id.clone(), p)))
        .collect();
    let outputs = futures::future::join_all(devices.into_iter().map(|device| {
        let pipe = pipes.get(&device.id).cloned();
        let cfg = cfg.clone();
        async move {
            let use_global = device
                .config
                .output
                .as_ref()
                .and_then(|o| o.use_global_audio)
                .unwrap_or(true);
            let version = pipe.as_ref().and_then(|p| p.audio_revision());
            let running = match pipe.as_ref() {
                Some(p) => p.metrics().await.ok().flatten().is_some_and(|m| {
                    m.running
                        && m.output
                            .iter()
                            .any(|o| o.stream.kind == "audio" && o.stream.traffic.packets_total > 0)
                }),
                None => false,
            };
            let state = if !cfg.enabled || !use_global {
                "original"
            } else if version == Some(cfg.version) && running {
                "applied"
            } else {
                "waiting"
            };
            OutputState {
                device_id: device.id,
                use_global_audio: use_global,
                effective_version: version.filter(|_| running),
                state,
            }
        }
    }))
    .await;
    Ok(ProgramState {
        config: cfg,
        outputs,
        last_error: LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    })
}

pub async fn initialize() -> anyhow::Result<()> {
    let conn = crate::db::app_db_conn()?;
    let saved: ProgramConfig = nvr_db::config::get_json(CONFIG_KEY, &conn)
        .await?
        .unwrap_or_default();
    saved.profile.validate()?;
    *CONFIG.write().unwrap_or_else(|e| e.into_inner()) = saved;
    super::restore_all().await;
    let stop = MONITOR_CANCEL.clone();
    let monitor = tokio::spawn(async move {
        let mut attached: HashMap<String, (Weak<Pipe>, Weak<nvr_audio_mixer::AudioSource>)> =
            HashMap::new();
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
        loop {
            tokio::select! { _ = stop.cancelled() => break, _ = tick.tick() => {} }
            let pipes: HashMap<_, _> = PIPES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter_map(|(id, p)| p.upgrade().map(|p| (id.clone(), p)))
                .collect();
            for id in super::MIXER.source_ids() {
                let Some(pipe) = pipes.get(&id) else {
                    continue;
                };
                let Some(source) = super::MIXER.source(&id) else {
                    continue;
                };
                if attached.get(&id).is_some_and(|(p, s)| {
                    p.upgrade().is_some_and(|p| Arc::ptr_eq(&p, pipe))
                        && s.upgrade().is_some_and(|s| Arc::ptr_eq(&s, &source))
                }) {
                    continue;
                }
                let receiver = tokio::select! {
                    _ = stop.cancelled() => return,
                    r = tokio::time::timeout(std::time::Duration::from_millis(300),pipe.subscribe_audio()) => r,
                };
                if let Ok(Ok(receiver)) = receiver {
                    if super::MIXER.attach_source(&id, receiver).is_ok() {
                        attached.insert(id, (Arc::downgrade(pipe), Arc::downgrade(&source)));
                    }
                }
            }
            attached.retain(|id, _| pipes.contains_key(id));
        }
    });
    *MONITOR.lock().unwrap_or_else(|e| e.into_inner()) = Some(monitor);
    Ok(())
}

pub async fn update(
    mut requested: ProgramConfig,
    hub: &'static crate::detect::hub::DetectHub,
) -> anyhow::Result<ProgramState> {
    // Control-plane serialization; packet processing never takes this lock.
    let _update = UPDATES.lock().await;
    let previous = config();
    anyhow::ensure!(
        requested.version == previous.version,
        "配置已被其他操作修改，请刷新后重试"
    );
    requested.profile.validate()?;
    if requested.enabled {
        let id = requested
            .active_bus_id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("请选择节目总线"))?;
        anyhow::ensure!(
            super::snapshot().buses.iter().any(|b| b.id == id),
            "节目总线不存在"
        );
    }
    requested.version = previous
        .version
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("配置版本溢出"))?;
    let buses = super::snapshot().buses;
    let conn = crate::db::app_db_conn()?;
    let devices = nvr_db::device::list(&conn).await?;
    super::MIXER.clear_device_buses().await;
    let applied: anyhow::Result<()> = async {
        if requested.profile != previous.profile {
            recreate_buses(&buses, &requested.profile).await?;
        }
        nvr_db::config::set_json(CONFIG_KEY, &requested, &conn).await?;
        *CONFIG.write().unwrap_or_else(|e| e.into_inner()) = requested.clone();
        apply_devices(&devices, hub).await
    }
    .await;
    if let Err(error) = applied {
        *CONFIG.write().unwrap_or_else(|e| e.into_inner()) = previous.clone();
        let restored: anyhow::Result<()> = async {
            super::MIXER.clear_device_buses().await;
            if requested.profile != previous.profile {
                recreate_buses(&buses, &previous.profile).await?;
            }
            nvr_db::config::set_json(CONFIG_KEY, &previous, &conn).await?;
            apply_devices(&devices, hub).await
        }
        .await;
        let message = match restored {
            Ok(()) => format!("{error:#}；已恢复原配置"),
            Err(e) => format!("{error:#}；恢复原配置也失败：{e:#}"),
        };
        *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = Some(message.clone());
        anyhow::bail!("{message}");
    }
    *LAST_ERROR.lock().unwrap_or_else(|e| e.into_inner()) = None;
    state().await
}

async fn recreate_buses(
    buses: &[nvr_audio_mixer::BusSnapshot],
    profile: &AudioProfile,
) -> anyhow::Result<()> {
    for bus in buses {
        let inputs: Vec<_> = bus
            .inputs
            .iter()
            .map(|i| (i.source_id.clone(), i.volume))
            .collect();
        super::MIXER
            .create_bus_with_profile(&bus.id, &bus.publish_url, &inputs, profile.clone())
            .await?;
        for input in &bus.inputs {
            super::MIXER.bus_set_muted(&bus.id, &input.source_id, input.muted)?;
            super::MIXER.bus_set_channel_map(&bus.id, &input.source_id, input.channel_map)?;
        }
    }
    Ok(())
}

pub(super) async fn apply_devices(
    devices: &[nvr_db::device::DeviceInfo],
    hub: &'static crate::detect::hub::DetectHub,
) -> anyhow::Result<()> {
    for device in devices {
        if device.input_type == "gb28181" {
            if let Some(bridge) = crate::gb::bridge() {
                bridge.refresh_program(&device.id).await?;
            }
        }
        crate::init::device::ensure_device_pipe(hub, device).await?;
    }
    Ok(())
}

pub async fn shutdown() {
    MONITOR_CANCEL.cancel();
    let monitor = MONITOR.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(monitor) = monitor {
        let _ = monitor.await;
    }
}
