//! ONVIF integration: a device_id -> OnvifConfig registry, the REST surface,
//! and the resolve-on-connect ingestion supervisor. Media reuses the existing
//! RTSP -> ZLM device pipeline; ONVIF only resolves the RTSP URI and drives PTZ.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use nvr_onvif::OnvifConfig;

// pub mod api; // added in Task 8
// pub mod ingest; // added in Task 9

/// device_id -> connection config, populated when an `onvif` device is added or
/// restored at startup. PTZ and stream re-resolution read from here.
static REGISTRY: LazyLock<RwLock<HashMap<String, OnvifConfig>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub(crate) fn register(device_id: &str, cfg: OnvifConfig) {
    REGISTRY.write().unwrap().insert(device_id.to_string(), cfg);
}

pub(crate) fn get(device_id: &str) -> Option<OnvifConfig> {
    REGISTRY.read().unwrap().get(device_id).cloned()
}

pub(crate) fn remove(device_id: &str) {
    REGISTRY.write().unwrap().remove(device_id);
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod mod_test;
