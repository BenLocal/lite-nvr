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
