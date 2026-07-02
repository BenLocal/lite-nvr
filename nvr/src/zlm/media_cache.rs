//! Live-stream registry fed by ZLM's `on_media_changed` hook. A stream key
//! `(app, stream)` is present iff ZLM currently has it registered (publishing).
//! Cheap-clone (Arc) so the hook and the API can share one instance.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct MediaCache {
    live: Arc<Mutex<HashSet<(String, String)>>>,
}

impl MediaCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// ZLM `on_media_changed` Regist: the stream is now publishing.
    pub fn on_regist(&self, app: &str, stream: &str) {
        self.live
            .lock()
            .unwrap()
            .insert((app.to_string(), stream.to_string()));
    }

    /// ZLM `on_media_changed` UnRegist: the stream stopped.
    pub fn on_unregist(&self, app: &str, stream: &str) {
        self.live
            .lock()
            .unwrap()
            .remove(&(app.to_string(), stream.to_string()));
    }

    pub fn is_live(&self, app: &str, stream: &str) -> bool {
        self.live
            .lock()
            .unwrap()
            .contains(&(app.to_string(), stream.to_string()))
    }

    pub fn live_streams(&self) -> Vec<(String, String)> {
        self.live.lock().unwrap().iter().cloned().collect()
    }
}

#[cfg(test)]
#[path = "media_cache_test.rs"]
mod media_cache_test;
