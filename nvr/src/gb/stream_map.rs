//! Pure mapping table: which ZLM stream id belongs to which GB device+channel.

use std::collections::HashMap;
use std::sync::Mutex;

/// The GB target a live ZLM stream pulls from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub device_id: String,
    pub channel_id: String,
}

/// Thread-safe stream_id → Mapping registry. `stream_id` is the ZLM stream name
/// (we use the nvr device id), independent of the ZLM app.
#[derive(Default)]
pub struct StreamMap {
    inner: Mutex<HashMap<String, Mapping>>,
}

impl StreamMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or overwrite the mapping for `stream_id`.
    pub fn register(&self, stream_id: &str, device_id: &str, channel_id: &str) {
        self.inner.lock().unwrap().insert(
            stream_id.to_string(),
            Mapping {
                device_id: device_id.to_string(),
                channel_id: channel_id.to_string(),
            },
        );
    }

    /// Remove a mapping. Returns the removed mapping, if any.
    pub fn unregister(&self, stream_id: &str) -> Option<Mapping> {
        self.inner.lock().unwrap().remove(stream_id)
    }

    pub fn get(&self, stream_id: &str) -> Option<Mapping> {
        self.inner.lock().unwrap().get(stream_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_get_unregister_roundtrip() {
        let m = StreamMap::new();
        assert!(m.get("cam1").is_none());
        m.register("cam1", "34020000001320000001", "34020000001320000002");
        assert_eq!(
            m.get("cam1").unwrap(),
            Mapping {
                device_id: "34020000001320000001".into(),
                channel_id: "34020000001320000002".into(),
            }
        );
        let removed = m.unregister("cam1").unwrap();
        assert_eq!(removed.channel_id, "34020000001320000002");
        assert!(m.get("cam1").is_none());
    }

    #[test]
    fn register_overwrites() {
        let m = StreamMap::new();
        m.register("cam1", "d1", "c1");
        m.register("cam1", "d1", "c2");
        assert_eq!(m.get("cam1").unwrap().channel_id, "c2");
    }
}
