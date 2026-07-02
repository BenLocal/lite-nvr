//! GB/T 28181 integration for nvr: a global on-demand bridge over the gb28181
//! crate's GbServer, wired to ZLM's media hooks. See
//! docs/superpowers/specs/2026-07-01-gb28181-crate-design.md row 11.

pub mod api;
pub mod bridge;
pub mod config;
pub mod receiver;
pub mod stream_map;

use std::sync::Arc;
use std::sync::OnceLock;

use gb28181::{GbEvent, GbServer};

use crate::gb::bridge::GbBridge;
use crate::gb::config::GbConfig;
use crate::gb::receiver::ZlmRtpReceiver;
use crate::zlm::cmd::ZlmControl;

static BRIDGE: OnceLock<Arc<GbBridge>> = OnceLock::new();

/// The global bridge, or `None` when GB support is disabled/uninitialized.
pub fn bridge() -> Option<Arc<GbBridge>> {
    BRIDGE.get().cloned()
}

/// Bind the GbServer and install the global bridge. Idempotent-safe: a second
/// call is ignored (returns Ok). No-op-friendly: callers gate on `GbConfig`.
pub async fn init(cfg: GbConfig) -> anyhow::Result<()> {
    if BRIDGE.get().is_some() {
        return Ok(());
    }
    let server_cfg = cfg.to_server_config()?;
    let (server, events) = GbServer::bind(server_cfg).await?;
    log::info!(
        "gb28181: platform listening on {} (id {})",
        server.local_addr(),
        cfg.sip_id
    );
    spawn_event_logger(events);
    let control = ZlmControl::spawn();
    let bridge = Arc::new(GbBridge::new(
        server,
        cfg.media_ip.clone(),
        Box::new(ZlmRtpReceiver::new(control)),
    ));
    let _ = BRIDGE.set(bridge);
    Ok(())
}

/// Drain GbServer events for observability (device online/offline, session end).
fn spawn_event_logger(mut events: tokio::sync::mpsc::UnboundedReceiver<GbEvent>) {
    tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            match e {
                GbEvent::Registered { device_id } => {
                    log::info!("gb28181: device registered: {device_id}")
                }
                GbEvent::Unregistered { device_id } => {
                    log::info!("gb28181: device unregistered: {device_id}")
                }
                GbEvent::Offline { device_id } => {
                    log::warn!("gb28181: device offline: {device_id}")
                }
                GbEvent::KeepaliveReceived { .. } => {}
                GbEvent::InviteReceived(_) => {} // server role never receives INVITE
                GbEvent::DeviceControlReceived { device_id, .. } => {
                    // Platform role never receives this (device role only); log defensively.
                    log::debug!("gb28181: unexpected DeviceControl from {device_id}")
                }
                GbEvent::SessionClosed { dialog_id } => {
                    log::info!("gb28181: session closed: {dialog_id}")
                }
            }
        }
    });
}
