//! Dynamic audio mixing console.
//!
//! A pool of [`AudioSource`]s (each a decoded audio input, decoded once and
//! shared) feeds any number of output [`MixBus`]es. Each bus mixes its own
//! selection of sources — with live per-input add/remove, volume and mute — and
//! publishes one independent stream. This is the audio analogue of
//! `nvr-compositor`: sources in, mixed program(s) out.
//!
//! ```text
//!   sources (device audio)        buses (independent outputs)
//!   ┌───────────┐
//!   │ cam-1     │──┐            ┌──────────────────────────┐
//!   ├───────────┤  ├─ vol/mute ─│ bus "hall"  → publish A  │
//!   │ cam-2     │──┤            └──────────────────────────┘
//!   ├───────────┤  │            ┌──────────────────────────┐
//!   │ cam-3     │──┴─ vol/mute ─│ bus "stream"→ publish B  │
//!   └───────────┘               └──────────────────────────┘
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

mod bus;
mod profile;
mod source;

pub use bus::{DEFAULT_VOLUME, MixBus};
pub use profile::{AudioCodec, AudioProfile, ChannelMap, MixMode};
pub use source::AudioSource;

/// One input's mix settings on a bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputSnapshot {
    pub source_id: String,
    /// Volume percent (100 = unity).
    pub volume: u32,
    pub muted: bool,
    #[serde(default)]
    pub channel_map: ChannelMap,
}

/// One output bus and its inputs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusSnapshot {
    pub id: String,
    pub publish_url: String,
    pub inputs: Vec<InputSnapshot>,
}

/// One source in the input pool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSnapshot {
    pub id: String,
    pub url: String,
    pub ready: bool,
    pub error: Option<String>,
}

/// Full mixer state, for the API and for persistence/restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixerSnapshot {
    pub sources: Vec<SourceSnapshot>,
    pub buses: Vec<BusSnapshot>,
}

/// The mixing console: owns the source pool and the output buses.
pub struct AudioMixer {
    sources: Mutex<HashMap<String, Arc<AudioSource>>>,
    buses: Mutex<HashMap<String, Arc<MixBus>>>,
    device_buses: Mutex<HashMap<(String, String), Arc<MixBus>>>,
    device_creation: tokio::sync::Mutex<()>,
    source_creation: tokio::sync::Mutex<()>,
}

impl AudioMixer {
    pub fn new() -> Self {
        Self {
            sources: Mutex::new(HashMap::new()),
            buses: Mutex::new(HashMap::new()),
            device_buses: Mutex::new(HashMap::new()),
            device_creation: tokio::sync::Mutex::new(()),
            source_creation: tokio::sync::Mutex::new(()),
        }
    }

    /// Ensure a source with `id` is in the pool, starting it from `url` if not
    /// already present. Idempotent.
    pub async fn ensure_source(&self, id: &str, url: &str) -> anyhow::Result<()> {
        if self.sources.lock().unwrap().contains_key(id) {
            return Ok(());
        }
        // Open + decode outside the lock (it may block on the network).
        let source = Arc::new(AudioSource::start(id, url).await?);
        self.sources
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_insert(source);
        Ok(())
    }

    /// Remove a source from the pool and detach it from every bus.
    pub fn remove_source(&self, id: &str) {
        self.sources.lock().unwrap().remove(id);
        for bus in self.buses.lock().unwrap().values() {
            let _ = bus.remove_input(id);
        }
    }

    pub fn has_source(&self, id: &str) -> bool {
        self.sources.lock().unwrap().contains_key(id)
    }

    pub fn ensure_shared_source(&self, id: &str) -> Arc<AudioSource> {
        let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
        sources
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(AudioSource::shared(id)))
            .clone()
    }

    pub fn attach_source(
        &self,
        id: &str,
        frames: ffmpeg_bus::frame::RawFrameReceiver,
    ) -> anyhow::Result<()> {
        self.ensure_shared_source(id).attach(frames)
    }

    pub async fn ensure_capture_source(
        &self,
        id: &str,
        address: &str,
        format: &str,
    ) -> anyhow::Result<()> {
        let _creation = self.source_creation.lock().await;
        if self.has_source(id) {
            return Ok(());
        }
        let source = Arc::new(AudioSource::capture(id, address, format));
        self.sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id.to_string())
            .or_insert(source);
        Ok(())
    }

    pub fn source(&self, id: &str) -> Option<Arc<AudioSource>> {
        self.sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    pub async fn prune_sources(&self) {
        let mut needed: std::collections::HashSet<String> = self
            .buses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .flat_map(|b| b.inputs_snapshot().into_iter().map(|i| i.source_id))
            .collect();
        needed.extend(
            self.device_buses
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .keys()
                .map(|(_, id)| id.clone()),
        );
        let removed = {
            let mut sources = self.sources.lock().unwrap_or_else(|e| e.into_inner());
            let keys: Vec<_> = sources
                .keys()
                .filter(|id| !needed.contains(*id))
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|id| sources.remove(&id))
                .collect::<Vec<_>>()
        };
        for source in removed {
            source.shutdown().await;
        }
    }

    pub async fn clear_device_buses(&self) {
        let buses: Vec<_> = self
            .device_buses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, bus)| bus)
            .collect();
        for bus in buses {
            bus.shutdown().await;
        }
    }

    pub fn source_ids(&self) -> Vec<String> {
        self.sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Create (or replace) an output bus that publishes to `publish_url`, mixing
    /// the given `(source_id, volume)` inputs. All referenced sources must
    /// already be in the pool (call [`ensure_source`](Self::ensure_source)
    /// first); the first input's stream is used as the encoder template, so at
    /// least one input is required.
    pub async fn create_bus(
        &self,
        bus_id: &str,
        publish_url: &str,
        inputs: &[(String, u32)],
    ) -> anyhow::Result<()> {
        self.create_bus_with_profile(bus_id, publish_url, inputs, AudioProfile::default())
            .await
    }

    pub async fn create_bus_with_profile(
        &self,
        bus_id: &str,
        publish_url: &str,
        inputs: &[(String, u32)],
        profile: AudioProfile,
    ) -> anyhow::Result<()> {
        profile.validate()?;
        self.remove_bus(bus_id).await;
        let bus = MixBus::start_program(bus_id, publish_url, profile).await?;
        {
            let sources = self.sources.lock().unwrap();
            for (source_id, volume) in inputs {
                if let Some(source) = sources.get(source_id) {
                    bus.add_input(source_id, source.subscribe(), *volume);
                } else {
                    log::warn!("audio bus '{bus_id}': source '{source_id}' not found, skipped");
                }
            }
        }
        // Replacing an existing id drops the old bus, which cancels its thread.
        self.buses
            .lock()
            .unwrap()
            .insert(bus_id.to_string(), Arc::new(bus));
        Ok(())
    }

    /// Remove and stop an output bus. Returns whether it existed.
    pub async fn remove_bus(&self, bus_id: &str) -> bool {
        let bus = self
            .buses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(bus_id);
        let variants = {
            let mut devices = self.device_buses.lock().unwrap_or_else(|e| e.into_inner());
            let keys: Vec<_> = devices
                .keys()
                .filter(|(bus, _)| bus == bus_id)
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|k| devices.remove(&k))
                .collect::<Vec<_>>()
        };
        for variant in variants {
            variant.shutdown().await;
        }
        if let Some(bus) = bus {
            bus.shutdown().await;
            true
        } else {
            false
        }
    }

    pub async fn output_audio(
        &self,
        bus_id: &str,
        device_id: &str,
        mode: MixMode,
        include_original: bool,
    ) -> anyhow::Result<ffmpeg_bus::shared_audio::SharedEncodedAudio> {
        // Serialize creation, not packet processing: two outputs joining the
        // same device concurrently must not briefly open two encoders.
        let _creation = self.device_creation.lock().await;
        let program = self
            .buses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(bus_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("program bus '{bus_id}' not found"))?;
        if mode == MixMode::Program || !include_original {
            return Ok(program.encoded());
        }
        let key = (bus_id.to_string(), device_id.to_string());
        if let Some(bus) = self
            .device_buses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
        {
            return Ok(bus.encoded());
        }
        let original = self.ensure_shared_source(device_id);
        let variant =
            Arc::new(MixBus::start_with_original(device_id, &program, original.subscribe()).await?);
        variant.set_muted(
            "original",
            program
                .inputs_snapshot()
                .iter()
                .any(|i| i.source_id == device_id),
        )?;
        let mut devices = self.device_buses.lock().unwrap_or_else(|e| e.into_inner());
        let bus = devices.entry(key).or_insert(variant);
        Ok(bus.encoded())
    }

    pub async fn shutdown(&self) {
        let ids: Vec<_> = self
            .buses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for id in ids {
            self.remove_bus(&id).await;
        }
        let sources: Vec<_> = self
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain()
            .map(|(_, source)| source)
            .collect();
        for source in sources {
            source.shutdown().await;
        }
    }

    /// Add an existing source to a bus at the given volume.
    pub fn bus_add_input(&self, bus_id: &str, source_id: &str, volume: u32) -> anyhow::Result<()> {
        let sources = self.sources.lock().unwrap();
        let source = sources
            .get(source_id)
            .ok_or_else(|| anyhow::anyhow!("source '{source_id}' not found"))?;
        let buses = self.buses.lock().unwrap();
        let bus = buses
            .get(bus_id)
            .ok_or_else(|| anyhow::anyhow!("bus '{bus_id}' not found"))?;
        bus.add_input(source_id, source.subscribe(), volume);
        for ((id, device), variant) in self
            .device_buses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if id == bus_id && device == source_id {
                variant.set_muted("original", true)?;
            }
        }
        Ok(())
    }

    pub fn bus_remove_input(&self, bus_id: &str, source_id: &str) -> anyhow::Result<()> {
        let buses = self.buses.lock().unwrap();
        let bus = buses
            .get(bus_id)
            .ok_or_else(|| anyhow::anyhow!("bus '{bus_id}' not found"))?;
        bus.remove_input(source_id)?;
        for ((id, device), variant) in self
            .device_buses
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if id == bus_id && device == source_id {
                variant.set_muted("original", false)?;
            }
        }
        Ok(())
    }

    pub fn bus_set_volume(&self, bus_id: &str, source_id: &str, volume: u32) -> anyhow::Result<()> {
        let buses = self.buses.lock().unwrap();
        let bus = buses
            .get(bus_id)
            .ok_or_else(|| anyhow::anyhow!("bus '{bus_id}' not found"))?;
        bus.set_volume(source_id, volume)
    }

    pub fn bus_set_muted(&self, bus_id: &str, source_id: &str, muted: bool) -> anyhow::Result<()> {
        let buses = self.buses.lock().unwrap();
        let bus = buses
            .get(bus_id)
            .ok_or_else(|| anyhow::anyhow!("bus '{bus_id}' not found"))?;
        bus.set_muted(source_id, muted)
    }

    pub fn bus_set_channel_map(
        &self,
        bus_id: &str,
        source_id: &str,
        mapping: ChannelMap,
    ) -> anyhow::Result<()> {
        let buses = self.buses.lock().unwrap_or_else(|e| e.into_inner());
        let bus = buses
            .get(bus_id)
            .ok_or_else(|| anyhow::anyhow!("bus not found"))?;
        bus.set_channel_map(source_id, mapping)
    }

    /// Snapshot the whole mixer for the API / persistence.
    pub fn snapshot(&self) -> MixerSnapshot {
        let sources = self
            .sources
            .lock()
            .unwrap()
            .values()
            .map(|s| SourceSnapshot {
                id: s.id.clone(),
                url: s.url.clone(),
                ready: s.ready(),
                error: s.error(),
            })
            .collect();
        let buses = self
            .buses
            .lock()
            .unwrap()
            .values()
            .map(|b| BusSnapshot {
                id: b.id().to_string(),
                publish_url: b.publish_url().to_string(),
                inputs: b.inputs_snapshot(),
            })
            .collect();
        MixerSnapshot { sources, buses }
    }
}

impl Default for AudioMixer {
    fn default() -> Self {
        Self::new()
    }
}
