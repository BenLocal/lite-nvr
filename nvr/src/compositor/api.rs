//! REST API for multi-view compositor programs. GET/POST only. Source URLs are
//! credential-redacted in responses.

use axum::{
    Json, Router,
    extract::Path,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::compositor::{self, CompositorEntry, CreateParams, SourceInfo};
use crate::handler::{ApiJsonResult, ok_empty, ok_json};
use nvr_compositor::Region;

pub fn compositor_router() -> Router {
    Router::new()
        .route("/create", post(create))
        .route("/list", get(list))
        .route("/switch/{id}", post(switch))
        .route("/remove/{id}", post(remove))
}

#[derive(Deserialize)]
struct SourceReq {
    id: String,
    url: String,
}

#[derive(Deserialize)]
struct RegionReq {
    /// The source id this region shows.
    source: String,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Deserialize)]
struct CreateReq {
    id: String,
    sources: Vec<SourceReq>,
    #[serde(default = "default_width")]
    width: u32,
    #[serde(default = "default_height")]
    height: u32,
    /// Explicit regions; omit for an automatic grid.
    #[serde(default)]
    regions: Vec<RegionReq>,
    #[serde(default = "default_fps")]
    fps: u32,
    #[serde(default)]
    bitrate: Option<u64>,
    #[serde(default)]
    publish_url: Option<String>,
}

fn default_width() -> u32 {
    1280
}
fn default_height() -> u32 {
    720
}
fn default_fps() -> u32 {
    25
}

#[derive(Serialize)]
struct SourceDto {
    id: String,
    url: String,
}

#[derive(Serialize)]
struct RegionDto {
    source: String,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Serialize)]
struct CompositorDto {
    id: String,
    sources: Vec<SourceDto>,
    width: u32,
    height: u32,
    regions: Vec<RegionDto>,
    publish_url: String,
    fps: u32,
}

fn to_dto(entry: &CompositorEntry) -> CompositorDto {
    CompositorDto {
        id: entry.id.clone(),
        sources: entry
            .sources
            .iter()
            .map(|s| SourceDto {
                id: s.id.clone(),
                url: redact_url(&s.url),
            })
            .collect(),
        width: entry.layout.width,
        height: entry.layout.height,
        // `source` is the region's *live* active source (may differ from the
        // initial layout after switching).
        regions: entry
            .layout
            .regions
            .iter()
            .zip(entry.active())
            .map(|(r, active)| RegionDto {
                source: active,
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            })
            .collect(),
        publish_url: entry.publish_url.clone(),
        fps: entry.fps,
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

async fn create(Json(req): Json<CreateReq>) -> ApiJsonResult<CompositorDto> {
    let params = CreateParams {
        id: req.id,
        sources: req
            .sources
            .into_iter()
            .map(|s| SourceInfo {
                id: s.id,
                url: s.url,
            })
            .collect(),
        width: req.width,
        height: req.height,
        regions: req
            .regions
            .into_iter()
            .map(|r| Region {
                source_id: r.source,
                x: r.x,
                y: r.y,
                w: r.w,
                h: r.h,
            })
            .collect(),
        fps: req.fps,
        bitrate: req.bitrate,
        publish_url: req.publish_url,
    };
    let entry = compositor::create(params).await?;
    Ok(ok_json(to_dto(&entry)))
}

async fn list() -> ApiJsonResult<Vec<CompositorDto>> {
    let items = compositor::list().await;
    Ok(ok_json(items.iter().map(|e| to_dto(e)).collect()))
}

#[derive(Deserialize)]
struct SwitchReq {
    /// Region index (by region order) to switch.
    region: usize,
    /// Source id (from the pool) to show in that region.
    to: String,
}

async fn switch(Path(id): Path<String>, Json(req): Json<SwitchReq>) -> ApiJsonResult<()> {
    compositor::switch(&id, req.region, &req.to).await?;
    Ok(ok_empty())
}

async fn remove(Path(id): Path<String>) -> ApiJsonResult<()> {
    if !compositor::remove(&id).await {
        return Err(anyhow::anyhow!("compositor {id} not found").into());
    }
    Ok(ok_empty())
}
