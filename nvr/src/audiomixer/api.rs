//! REST API for the audio mixing console. GET/POST only. URLs are
//! credential-redacted in responses.

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::audiomixer;
use crate::handler::{ApiJsonResult, ok_empty, ok_json};
use nvr_audio_mixer::{DEFAULT_VOLUME, MixerSnapshot};

pub fn audiomixer_router(hub: &'static crate::detect::hub::DetectHub) -> Router {
    Router::new()
        .route("/list", get(list))
        .route("/bus/create", post(create_bus))
        .route("/bus/remove", post(remove_bus))
        .route("/bus/input/add", post(add_input))
        .route("/bus/input/remove", post(remove_input))
        .route("/bus/input/volume", post(set_volume))
        .route("/bus/input/mute", post(set_muted))
        .route("/config", get(config).post(set_config))
        .route("/sources", get(sources))
        .route("/bus/input/channels", post(set_channels))
        .with_state(hub)
}

async fn sources() -> ApiJsonResult<Vec<super::sources::SourceOption>> {
    Ok(ok_json(super::sources::available().await?))
}

async fn config() -> ApiJsonResult<super::output::ProgramState> {
    Ok(ok_json(super::output::state().await?))
}

async fn set_config(
    State(hub): State<&'static crate::detect::hub::DetectHub>,
    Json(config): Json<super::output::ProgramConfig>,
) -> ApiJsonResult<super::output::ProgramState> {
    Ok(ok_json(super::output::update(config, hub).await?))
}

// ---- responses -------------------------------------------------------------

#[derive(Serialize)]
struct InputDto {
    source_id: String,
    volume: u32,
    muted: bool,
    channel_map: nvr_audio_mixer::ChannelMap,
}

#[derive(Serialize)]
struct BusDto {
    id: String,
    publish_url: String,
    /// FLV path for playing the mixed output on the dashboard.
    flv_url: String,
    inputs: Vec<InputDto>,
}

#[derive(Serialize)]
struct SourceDto {
    id: String,
    url: String,
    ready: bool,
    error: Option<String>,
}

#[derive(Serialize)]
struct MixerDto {
    sources: Vec<SourceDto>,
    buses: Vec<BusDto>,
}

fn to_dto(snap: MixerSnapshot) -> MixerDto {
    MixerDto {
        sources: snap
            .sources
            .into_iter()
            .map(|s| SourceDto {
                id: s.id,
                url: redact_url(&s.url),
                ready: s.ready,
                error: s.error,
            })
            .collect(),
        buses: snap
            .buses
            .into_iter()
            .map(|b| BusDto {
                flv_url: audiomixer::bus_flv_url(&b.id),
                publish_url: redact_url(&b.publish_url),
                id: b.id,
                inputs: b
                    .inputs
                    .into_iter()
                    .map(|i| InputDto {
                        source_id: i.source_id,
                        volume: i.volume,
                        muted: i.muted,
                        channel_map: i.channel_map,
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Strip `user:pass@` credentials from a URL's authority for safe display.
fn redact_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let rest = &url[scheme_end + 3..];
    let auth_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..auth_end];
    match authority.rfind('@') {
        Some(at) => format!(
            "{}://{}{}",
            &url[..scheme_end],
            &authority[at + 1..],
            &rest[auth_end..]
        ),
        None => url.to_string(),
    }
}

// ---- requests --------------------------------------------------------------

#[derive(Deserialize)]
struct InputReq {
    /// Device id to mix in.
    source_id: String,
    #[serde(default)]
    volume: Option<u32>,
}

#[derive(Deserialize)]
struct CreateBusReq {
    id: String,
    #[serde(default)]
    publish_url: Option<String>,
    inputs: Vec<InputReq>,
}

#[derive(Deserialize)]
struct BusRef {
    bus_id: String,
}

#[derive(Deserialize)]
struct AddInputReq {
    bus_id: String,
    source_id: String,
    #[serde(default)]
    volume: Option<u32>,
}

#[derive(Deserialize)]
struct InputRef {
    bus_id: String,
    source_id: String,
}

#[derive(Deserialize)]
struct VolumeReq {
    bus_id: String,
    source_id: String,
    volume: u32,
}

#[derive(Deserialize)]
struct MuteReq {
    bus_id: String,
    source_id: String,
    muted: bool,
}

#[derive(Deserialize)]
struct ChannelReq {
    bus_id: String,
    source_id: String,
    channel_map: nvr_audio_mixer::ChannelMap,
}

async fn set_channels(Json(req): Json<ChannelReq>) -> ApiJsonResult<()> {
    audiomixer::set_channel_map(&req.bus_id, &req.source_id, req.channel_map).await?;
    Ok(ok_empty())
}

// ---- handlers --------------------------------------------------------------

async fn list() -> ApiJsonResult<MixerDto> {
    Ok(ok_json(to_dto(audiomixer::snapshot())))
}

async fn create_bus(Json(req): Json<CreateBusReq>) -> ApiJsonResult<MixerDto> {
    let inputs = req
        .inputs
        .into_iter()
        .map(|i| (i.source_id, i.volume.unwrap_or(DEFAULT_VOLUME)))
        .collect();
    audiomixer::create_bus(&req.id, req.publish_url, inputs).await?;
    Ok(ok_json(to_dto(audiomixer::snapshot())))
}

async fn remove_bus(Json(req): Json<BusRef>) -> ApiJsonResult<()> {
    audiomixer::remove_bus(&req.bus_id).await?;
    Ok(ok_empty())
}

async fn add_input(Json(req): Json<AddInputReq>) -> ApiJsonResult<()> {
    audiomixer::add_input(
        &req.bus_id,
        &req.source_id,
        req.volume.unwrap_or(DEFAULT_VOLUME),
    )
    .await?;
    Ok(ok_empty())
}

async fn remove_input(Json(req): Json<InputRef>) -> ApiJsonResult<()> {
    audiomixer::remove_input(&req.bus_id, &req.source_id).await?;
    Ok(ok_empty())
}

async fn set_volume(Json(req): Json<VolumeReq>) -> ApiJsonResult<()> {
    audiomixer::set_volume(&req.bus_id, &req.source_id, req.volume).await?;
    Ok(ok_empty())
}

async fn set_muted(Json(req): Json<MuteReq>) -> ApiJsonResult<()> {
    audiomixer::set_muted(&req.bus_id, &req.source_id, req.muted).await?;
    Ok(ok_empty())
}
