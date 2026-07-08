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
mod source;

pub use bus::{DEFAULT_VOLUME, MixBus};
pub use source::AudioSource;

/// One input's mix settings on a bus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputSnapshot {
    pub source_id: String,
    /// Volume percent (100 = unity).
    pub volume: u32,
    pub muted: bool,
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
    buses: Mutex<HashMap<String, MixBus>>,
}

impl AudioMixer {
    pub fn new() -> Self {
        Self {
            sources: Mutex::new(HashMap::new()),
            buses: Mutex::new(HashMap::new()),
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
        // Resolve the encoder template without holding the lock across the await.
        let template = {
            let sources = self.sources.lock().unwrap();
            let (first_id, _) = inputs
                .first()
                .ok_or_else(|| anyhow::anyhow!("bus '{bus_id}' needs at least one input"))?;
            sources
                .get(first_id)
                .ok_or_else(|| anyhow::anyhow!("source '{first_id}' not found"))?
                .audio_stream
                .clone()
        };

        let bus = MixBus::start(bus_id, publish_url, template).await?;
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
        self.buses.lock().unwrap().insert(bus_id.to_string(), bus);
        Ok(())
    }

    /// Remove and stop an output bus. Returns whether it existed.
    pub fn remove_bus(&self, bus_id: &str) -> bool {
        self.buses.lock().unwrap().remove(bus_id).is_some()
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
        Ok(())
    }

    pub fn bus_remove_input(&self, bus_id: &str, source_id: &str) -> anyhow::Result<()> {
        let buses = self.buses.lock().unwrap();
        let bus = buses
            .get(bus_id)
            .ok_or_else(|| anyhow::anyhow!("bus '{bus_id}' not found"))?;
        bus.remove_input(source_id)
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
