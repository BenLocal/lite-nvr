//! The on-demand bridge: owns the GbServer, the stream map, and the live
//! sessions. ZLM hooks call `handle_media_not_found`/`handle_media_no_reader`.

use std::collections::HashMap;
use std::sync::Mutex;

use gb28181::{GbServer, MediaSession, MediaSpec, SsrcKind, StreamType, Transport};

use crate::gb::receiver::{MediaReceiver, ReceiverHandle};
use crate::gb::stream_map::StreamMap;

/// A live pull: the ZLM receiver (kept alive) plus the GB media dialog.
struct ActiveSession {
    _receiver: Box<dyn ReceiverHandle>,
    session: MediaSession,
}

pub struct GbBridge {
    server: GbServer,
    media_ip: String,
    receiver: Box<dyn MediaReceiver>,
    streams: StreamMap,
    active: Mutex<HashMap<String, ActiveSession>>,
}

impl GbBridge {
    pub fn new(server: GbServer, media_ip: String, receiver: Box<dyn MediaReceiver>) -> Self {
        Self {
            server,
            media_ip,
            receiver,
            streams: StreamMap::new(),
            active: Mutex::new(HashMap::new()),
        }
    }

    pub fn server(&self) -> &GbServer {
        &self.server
    }

    pub fn register_mapping(&self, stream_id: &str, device_id: &str, channel_id: &str) {
        self.streams.register(stream_id, device_id, channel_id);
    }

    /// Remove the mapping and tear down any live session for it.
    pub async fn unregister_mapping(&self, stream_id: &str) {
        self.streams.unregister(stream_id);
        self.teardown(stream_id).await;
    }

    /// ZLM `on_media_not_found`: pull the stream if it's a known gb mapping and
    /// not already active. Idempotent — ZLM may fire this repeatedly. Returns
    /// true iff this bridge recognizes and is handling the stream.
    pub async fn handle_media_not_found(&self, stream_id: &str) -> bool {
        let Some(mapping) = self.streams.get(stream_id) else {
            return false;
        };
        if self.active.lock().unwrap().contains_key(stream_id) {
            return true; // already pulling
        }
        if let Err(e) = self.start_pull(stream_id, &mapping).await {
            log::error!("gb28181: pull for stream {stream_id} failed: {e:#}");
        }
        true
    }

    async fn start_pull(
        &self,
        stream_id: &str,
        mapping: &crate::gb::stream_map::Mapping,
    ) -> anyhow::Result<()> {
        let handle = self.receiver.open(stream_id, Transport::Udp)?;
        let port = handle.port();
        let (ssrc, ssrc_str) = self.server.next_ssrc(SsrcKind::Live);
        let media_addr = format!("{}:{}", self.media_ip, port).parse()?;
        let spec = MediaSpec {
            ssrc,
            ssrc_str,
            transport: Transport::Udp,
            media_addr,
            stream_type: StreamType::Play,
            negotiated_remote: None,
        };
        let session = self
            .server
            .invite_play(&mapping.device_id, &mapping.channel_id, spec)
            .await?;
        self.active.lock().unwrap().insert(
            stream_id.to_string(),
            ActiveSession {
                _receiver: handle,
                session,
            },
        );
        log::info!(
            "gb28181: pulling {} channel {} -> stream {} (port {})",
            mapping.device_id,
            mapping.channel_id,
            stream_id,
            port
        );
        Ok(())
    }

    /// ZLM `on_media_no_reader`: last viewer left — BYE and release.
    pub async fn handle_media_no_reader(&self, stream_id: &str) {
        self.teardown(stream_id).await;
    }

    async fn teardown(&self, stream_id: &str) {
        let active = self.active.lock().unwrap().remove(stream_id);
        if let Some(active) = active {
            if let Err(e) = active.session.stop().await {
                log::warn!("gb28181: BYE for stream {stream_id} failed: {e:#}");
            }
            log::info!("gb28181: released stream {stream_id}");
        }
        // `_receiver` drops here, releasing the ZLM RtpServer port.
    }

    #[cfg(test)]
    pub(crate) fn is_active(&self, stream_id: &str) -> bool {
        self.active.lock().unwrap().contains_key(stream_id)
    }
}

#[cfg(test)]
#[path = "bridge_test.rs"]
mod bridge_test;
