//! REST API for director/switcher programs. GET/POST only (dashboard
//! convention). Source URLs are credential-redacted in responses.

use axum::{
    Json, Router,
    extract::Path,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::handler::{ApiJsonResult, ok_empty, ok_json};
use crate::program::{self, CreateParams, ProgramEntry, SourceInfo};

pub fn program_router() -> Router {
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
struct CreateReq {
    id: String,
    sources: Vec<SourceReq>,
    #[serde(default = "default_fps")]
    fps: u32,
    #[serde(default)]
    bitrate: Option<u64>,
    #[serde(default)]
    publish_url: Option<String>,
}

fn default_fps() -> u32 {
    25
}

#[derive(Deserialize)]
struct SwitchReq {
    to: String,
}

#[derive(Serialize)]
struct SourceDto {
    id: String,
    url: String,
}

#[derive(Serialize)]
struct ProgramDto {
    id: String,
    sources: Vec<SourceDto>,
    active: String,
    publish_url: String,
    fps: u32,
}

fn to_dto(entry: &ProgramEntry) -> ProgramDto {
    ProgramDto {
        id: entry.id.clone(),
        sources: entry
            .sources
            .iter()
            .map(|s| SourceDto {
                id: s.id.clone(),
                url: redact_url(&s.url),
            })
            .collect(),
        active: entry.active(),
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

async fn create(Json(req): Json<CreateReq>) -> ApiJsonResult<ProgramDto> {
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
        fps: req.fps,
        bitrate: req.bitrate,
        publish_url: req.publish_url,
    };
    let entry = program::create(params).await?;
    Ok(ok_json(to_dto(&entry)))
}

async fn list() -> ApiJsonResult<Vec<ProgramDto>> {
    let items = program::list().await;
    Ok(ok_json(items.iter().map(|e| to_dto(e)).collect()))
}

async fn switch(Path(id): Path<String>, Json(req): Json<SwitchReq>) -> ApiJsonResult<()> {
    program::switch(&id, req.to.trim()).await?;
    Ok(ok_empty())
}

async fn remove(Path(id): Path<String>) -> ApiJsonResult<()> {
    if !program::remove(&id).await {
        return Err(anyhow::anyhow!("program {id} not found").into());
    }
    Ok(ok_empty())
}
