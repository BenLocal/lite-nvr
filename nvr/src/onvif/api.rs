use axum::{
    Json, Router,
    extract::Path,
    routing::{get, post},
};
use nvr_onvif::{Discovered, OnvifCamera, OnvifConfig, Preset, Profile, PtzVelocity, discover};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::handler::{ApiJsonResult, ok_empty, ok_json};

pub fn onvif_router() -> Router {
    Router::new()
        .route("/discover", post(discover_handler))
        .route("/probe", post(probe))
        .route("/ptz", post(ptz))
        .route("/presets/{device_id}", get(presets))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PtzAction {
    Move(PtzVelocity),
    Stop,
    Preset(String),
}

/// Map the gb-style direction verb + speed (0..=255) to a PTZ action.
/// Returns None for an unknown verb or `preset_call` without a token.
pub(crate) fn resolve_ptz(
    direction: &str,
    speed: u8,
    preset_token: Option<&str>,
) -> Option<PtzAction> {
    let s = speed as f32 / 255.0;
    let mv =
        |pan: f32, tilt: f32, zoom: f32| Some(PtzAction::Move(PtzVelocity::new(pan, tilt, zoom)));
    match direction {
        "up" => mv(0.0, s, 0.0),
        "down" => mv(0.0, -s, 0.0),
        "left" => mv(-s, 0.0, 0.0),
        "right" => mv(s, 0.0, 0.0),
        "zoom_in" => mv(0.0, 0.0, s),
        "zoom_out" => mv(0.0, 0.0, -s),
        "stop" => Some(PtzAction::Stop),
        "preset_call" => preset_token.map(|t| PtzAction::Preset(t.to_string())),
        _ => None,
    }
}

#[derive(Deserialize)]
struct DiscoverRequest {
    timeout_ms: Option<u64>,
}

async fn discover_handler(Json(req): Json<DiscoverRequest>) -> ApiJsonResult<Vec<Discovered>> {
    let timeout = Duration::from_millis(req.timeout_ms.unwrap_or(3000).clamp(500, 10_000));
    let found = discover(timeout)
        .await
        .map_err(|e| anyhow::anyhow!("onvif discover: {e}"))?;
    Ok(ok_json(found))
}

#[derive(Deserialize)]
struct ProbeRequest {
    host: String,
    port: u16,
    username: String,
    password: String,
}

#[derive(Serialize)]
struct ProbeResponse {
    device_info: nvr_onvif::DeviceInfo,
    profiles: Vec<Profile>,
}

async fn probe(Json(req): Json<ProbeRequest>) -> ApiJsonResult<ProbeResponse> {
    let cfg = OnvifConfig {
        host: req.host,
        port: req.port,
        username: req.username,
        password: req.password,
        profile_token: None,
    };
    let cam = OnvifCamera::connect(&cfg)
        .await
        .map_err(|e| anyhow::anyhow!("onvif connect: {e}"))?;
    let device_info = cam
        .device_info()
        .await
        .map_err(|e| anyhow::anyhow!("onvif device_info: {e}"))?;
    let profiles = cam
        .profiles()
        .await
        .map_err(|e| anyhow::anyhow!("onvif profiles: {e}"))?;
    Ok(ok_json(ProbeResponse {
        device_info,
        profiles,
    }))
}

#[derive(Deserialize)]
struct PtzRequest {
    device_id: String,
    direction: String,
    speed: Option<u8>,
    preset_token: Option<String>,
}

async fn ptz(Json(req): Json<PtzRequest>) -> ApiJsonResult<()> {
    let cfg = super::get(&req.device_id)
        .ok_or_else(|| anyhow::anyhow!("no onvif device: {}", req.device_id))?;
    let action = resolve_ptz(
        &req.direction,
        req.speed.unwrap_or(128),
        req.preset_token.as_deref(),
    )
    .ok_or_else(|| anyhow::anyhow!("bad ptz direction: {}", req.direction))?;

    let cam = OnvifCamera::connect(&cfg)
        .await
        .map_err(|e| anyhow::anyhow!("onvif connect: {e}"))?;
    match action {
        PtzAction::Move(v) => cam.ptz_move(v).await,
        PtzAction::Stop => cam.ptz_stop().await,
        PtzAction::Preset(t) => cam.goto_preset(&t).await,
    }
    .map_err(|e| anyhow::anyhow!("onvif ptz: {e}"))?;
    Ok(ok_empty())
}

async fn presets(Path(device_id): Path<String>) -> ApiJsonResult<Vec<Preset>> {
    let cfg =
        super::get(&device_id).ok_or_else(|| anyhow::anyhow!("no onvif device: {device_id}"))?;
    let cam = OnvifCamera::connect(&cfg)
        .await
        .map_err(|e| anyhow::anyhow!("onvif connect: {e}"))?;
    let presets = cam
        .presets()
        .await
        .map_err(|e| anyhow::anyhow!("onvif presets: {e}"))?;
    Ok(ok_json(presets))
}

#[cfg(test)]
#[path = "api_test.rs"]
mod api_test;
