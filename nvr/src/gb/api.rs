//! Read-only API over the live GB registrar/catalog, for the dashboard picker.

use axum::{
    Json, Router,
    extract::Path,
    routing::{get, post},
};
use gb28181::PtzCommand;
use serde::{Deserialize, Serialize};

use crate::handler::{ApiJsonResult, ok_empty, ok_json};

pub fn gb_router() -> Router {
    Router::new()
        .route("/devices", get(list_devices))
        .route("/catalog/{device_id}", get(catalog))
        .route("/ptz", post(ptz))
        .route("/play", post(play))
        .route("/streams", get(streams))
}

#[derive(Serialize)]
struct GbDeviceItem {
    device_id: String,
    online: bool,
}

#[derive(Serialize)]
struct GbChannelItem {
    channel_id: String,
    name: String,
    status: String,
}

/// Devices currently in the platform registrar. Empty when GB is disabled.
async fn list_devices() -> ApiJsonResult<Vec<GbDeviceItem>> {
    let Some(bridge) = crate::gb::bridge() else {
        return Ok(ok_json(Vec::new()));
    };
    let items = bridge
        .server()
        .devices()
        .into_iter()
        .map(|d| GbDeviceItem {
            device_id: d.device_id,
            online: d.online,
        })
        .collect();
    Ok(ok_json(items))
}

/// Query a device's channel catalog (live MANSCDP Catalog). Returns an empty
/// list (HTTP 200) when GB is disabled; propagates crate errors otherwise.
async fn catalog(Path(device_id): Path<String>) -> ApiJsonResult<Vec<GbChannelItem>> {
    let Some(bridge) = crate::gb::bridge() else {
        return Ok(ok_json(Vec::new()));
    };
    let catalog = bridge.server().catalog_query(&device_id).await?;
    let items = catalog
        .items
        .into_iter()
        .map(|c| GbChannelItem {
            channel_id: c.device_id,
            name: c.name,
            status: c.status,
        })
        .collect();
    Ok(ok_json(items))
}

#[derive(Deserialize)]
struct PtzRequest {
    device_id: String,
    channel_id: String,
    /// One of: up, down, left, right, up_left, up_right, down_left, down_right,
    /// zoom_in, zoom_out, stop, preset_call, preset_set, preset_delete.
    command: String,
    /// Movement speed 0..=255 (pan/tilt); zoom uses the low 4 bits. Default 128.
    #[serde(default)]
    speed: Option<u8>,
    /// Preset number 1..=255, for the preset_* commands.
    #[serde(default)]
    preset: Option<u8>,
}

/// Map a request into a `PtzCommand`, or `None` for an unknown command.
fn to_ptz(req: &PtzRequest) -> Option<PtzCommand> {
    let speed = req.speed.unwrap_or(128);
    // A set zoom bit must carry a non-zero speed, else the lens won't move —
    // clamp to 1 so a low slider value still zooms (it already pans/tilts).
    let zoom_speed = (speed >> 4).max(1); // 1..=15
    let mv = |up, down, left, right, zoom_in, zoom_out| PtzCommand::Move {
        up,
        down,
        left,
        right,
        zoom_in,
        zoom_out,
        pan_speed: if left || right { speed } else { 0 },
        tilt_speed: if up || down { speed } else { 0 },
        zoom_speed: if zoom_in || zoom_out { zoom_speed } else { 0 },
    };
    Some(match req.command.as_str() {
        "up" => mv(true, false, false, false, false, false),
        "down" => mv(false, true, false, false, false, false),
        "left" => mv(false, false, true, false, false, false),
        "right" => mv(false, false, false, true, false, false),
        "up_left" => mv(true, false, true, false, false, false),
        "up_right" => mv(true, false, false, true, false, false),
        "down_left" => mv(false, true, true, false, false, false),
        "down_right" => mv(false, true, false, true, false, false),
        "zoom_in" => mv(false, false, false, false, true, false),
        "zoom_out" => mv(false, false, false, false, false, true),
        "stop" => PtzCommand::stop(),
        "preset_call" => PtzCommand::PresetCall(req.preset?),
        "preset_set" => PtzCommand::PresetSet(req.preset?),
        "preset_delete" => PtzCommand::PresetDelete(req.preset?),
        _ => return None,
    })
}

/// Send a PTZ / DeviceControl command to a gb device's channel.
async fn ptz(Json(req): Json<PtzRequest>) -> ApiJsonResult<()> {
    let Some(bridge) = crate::gb::bridge() else {
        return Err(anyhow::anyhow!("GB support is not enabled").into());
    };
    let cmd =
        to_ptz(&req).ok_or_else(|| anyhow::anyhow!("unknown ptz command: {}", req.command))?;
    bridge
        .server()
        .device_control(&req.device_id, &req.channel_id, cmd)
        .await?;
    Ok(ok_empty())
}

#[derive(Deserialize)]
struct PlayRequest {
    /// The nvr device id (== ZLM stream id).
    device_id: String,
    /// "udp" | "tcp_passive" | "tcp_active"; defaults to "udp" when omitted.
    #[serde(default)]
    transport: Option<String>,
}

#[derive(Serialize)]
struct PlayResponse {
    stream_id: String,
    url: String,
}

/// Parse the transport string; `None` (missing) defaults to Udp; an unknown
/// value yields `None` (rejected by the handler).
fn parse_transport(s: Option<&str>) -> Option<gb28181::Transport> {
    match s {
        None | Some("udp") => Some(gb28181::Transport::Udp),
        Some("tcp_passive") => Some(gb28181::Transport::TcpPassive),
        Some("tcp_active") => Some(gb28181::Transport::TcpActive),
        Some(_) => None,
    }
}

/// Set the transport for a configured gb stream and return its playable URL.
/// `device_id` is the nvr device id (== stream id); the mapping must already
/// exist (registered from the gb device config at startup).
async fn play(Json(req): Json<PlayRequest>) -> ApiJsonResult<PlayResponse> {
    let Some(bridge) = crate::gb::bridge() else {
        return Err(anyhow::anyhow!("GB support is not enabled").into());
    };
    let transport = parse_transport(req.transport.as_deref())
        .ok_or_else(|| anyhow::anyhow!("unknown transport: {:?}", req.transport))?;
    if !bridge.set_transport(&req.device_id, transport) {
        return Err(anyhow::anyhow!("no gb stream mapping for device {}", req.device_id).into());
    }
    Ok(ok_json(PlayResponse {
        stream_id: req.device_id.clone(),
        url: crate::init::device::build_gb_flv_url(&req.device_id),
    }))
}

#[derive(Serialize)]
struct StreamStatusDto {
    stream_id: String,
    device_id: String,
    channel_id: String,
    transport: String,
    live: bool,
    rtp: Option<RtpInfoDto>,
}

#[derive(Serialize)]
struct RtpInfoDto {
    exist: bool,
    peer_ip: String,
    peer_port: u16,
    local_port: u16,
    identifier: String,
}

fn transport_str(t: gb28181::Transport) -> &'static str {
    match t {
        gb28181::Transport::Udp => "udp",
        gb28181::Transport::TcpPassive => "tcp_passive",
        gb28181::Transport::TcpActive => "tcp_active",
    }
}

/// Live status of every gb stream mapping (empty list when GB is disabled).
async fn streams() -> ApiJsonResult<Vec<StreamStatusDto>> {
    let Some(bridge) = crate::gb::bridge() else {
        return Ok(ok_json(Vec::new()));
    };
    let items = bridge
        .stream_status()
        .await
        .into_iter()
        .map(|s| StreamStatusDto {
            stream_id: s.stream_id,
            device_id: s.device_id,
            channel_id: s.channel_id,
            transport: transport_str(s.transport).to_string(),
            live: s.live,
            rtp: s.rtp.map(|r| RtpInfoDto {
                exist: r.exist,
                peer_ip: r.peer_ip,
                peer_port: r.peer_port,
                local_port: r.local_port,
                identifier: r.identifier,
            }),
        })
        .collect();
    Ok(ok_json(items))
}

#[cfg(test)]
mod play_tests {
    use super::*;

    #[test]
    fn parse_transport_maps_known_values_and_defaults() {
        assert_eq!(parse_transport(None), Some(gb28181::Transport::Udp));
        assert_eq!(parse_transport(Some("udp")), Some(gb28181::Transport::Udp));
        assert_eq!(
            parse_transport(Some("tcp_passive")),
            Some(gb28181::Transport::TcpPassive)
        );
        assert_eq!(
            parse_transport(Some("tcp_active")),
            Some(gb28181::Transport::TcpActive)
        );
        assert_eq!(parse_transport(Some("bogus")), None);
    }
}
