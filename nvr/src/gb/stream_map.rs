//! Pure mapping table: which ZLM stream id belongs to which GB device+channel.

use std::collections::HashMap;
use std::sync::Mutex;

use gb28181::Transport;

/// The GB target a live ZLM stream pulls from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub device_id: String,
    pub channel_id: String,
    pub transport: Transport,
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
    pub fn register(
        &self,
        stream_id: &str,
        device_id: &str,
        channel_id: &str,
        transport: Transport,
    ) {
        self.inner.lock().unwrap().insert(
            stream_id.to_string(),
            Mapping {
                device_id: device_id.to_string(),
                channel_id: channel_id.to_string(),
                transport,
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

    /// Update the transport of an existing mapping. Returns false if absent.
    pub fn set_transport(&self, stream_id: &str, transport: Transport) -> bool {
        match self.inner.lock().unwrap().get_mut(stream_id) {
            Some(m) => {
                m.transport = transport;
                true
            }
            None => false,
        }
    }

    /// Snapshot of all (stream_id, mapping) pairs.
    pub fn list(&self) -> Vec<(String, Mapping)> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_get_unregister_roundtrip() {
        let m = StreamMap::new();
        assert!(m.get("cam1").is_none());
        m.register(
            "cam1",
            "34020000001320000001",
            "34020000001320000002",
            Transport::Udp,
        );
        assert_eq!(
            m.get("cam1").unwrap(),
            Mapping {
                device_id: "34020000001320000001".into(),
                channel_id: "34020000001320000002".into(),
                transport: Transport::Udp,
            }
        );
        let removed = m.unregister("cam1").unwrap();
        assert_eq!(removed.channel_id, "34020000001320000002");
        assert!(m.get("cam1").is_none());
    }

    #[test]
    fn register_overwrites() {
        let m = StreamMap::new();
        m.register("cam1", "d1", "c1", Transport::Udp);
        m.register("cam1", "d1", "c2", Transport::Udp);
        assert_eq!(m.get("cam1").unwrap().channel_id, "c2");
    }

    #[test]
    fn register_stores_transport_and_set_transport_updates() {
        use gb28181::Transport;
        let m = StreamMap::new();
        m.register("cam1", "d1", "c1", Transport::Udp);
        assert_eq!(m.get("cam1").unwrap().transport, Transport::Udp);
        assert!(m.set_transport("cam1", Transport::TcpActive));
        assert_eq!(m.get("cam1").unwrap().transport, Transport::TcpActive);
        assert!(!m.set_transport("nope", Transport::TcpActive));
    }

    #[test]
    fn list_returns_all_mappings() {
        use gb28181::Transport;
        let m = StreamMap::new();
        m.register("cam1", "d1", "c1", Transport::Udp);
        m.register("cam2", "d2", "c2", Transport::TcpPassive);
        let mut ids: Vec<String> = m.list().into_iter().map(|(id, _)| id).collect();
        ids.sort();
        assert_eq!(ids, vec!["cam1".to_string(), "cam2".to_string()]);
    }
}
