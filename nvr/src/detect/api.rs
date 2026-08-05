//! Detection control + read endpoints. Opt-in start/stop per pipe; GET latest
//! per-frame multi-model result. GET/POST only; session auth is applied by the
//! parent `/api` router.

use axum::{
    Json, Router,
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;

use super::hub::DetectHub;

#[derive(Deserialize, Default)]
pub struct StartBody {
    /// Subset of configured model names to run. Absent/empty = all.
    #[serde(default)]
    pub models: Option<Vec<String>>,
}

pub fn detect_router() -> Router {
    Router::new()
        .route("/{pipe}/start", post(start))
        .route("/{pipe}/stop", post(stop))
        .route("/{pipe}/latest", get(latest))
        .route("/models", get(models))
}

async fn models() -> impl IntoResponse {
    let Some(hub) = DetectHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "detect not initialized").into_response();
    };
    Json(hub.config_names()).into_response()
}

async fn latest(Path(pipe): Path<String>) -> impl IntoResponse {
    let Some(hub) = DetectHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "detect not initialized").into_response();
    };
    match hub.latest(&pipe) {
        Some(fr) => Json(fr).into_response(),
        None => (StatusCode::NOT_FOUND, "no result yet").into_response(),
    }
}

async fn stop(Path(pipe): Path<String>) -> impl IntoResponse {
    let Some(hub) = DetectHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "detect not initialized").into_response();
    };
    if hub.unregister(&pipe) {
        (StatusCode::OK, "stopped").into_response()
    } else {
        (StatusCode::OK, "not running").into_response()
    }
}

async fn start(Path(pipe): Path<String>, body: Option<Json<StartBody>>) -> impl IntoResponse {
    let Some(hub) = DetectHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "detect not initialized").into_response();
    };
    let want = body.and_then(|Json(b)| b.models);
    match crate::detect::control::start_tap(hub, &pipe, want, 0, 0.0, None).await {
        Ok(crate::detect::control::StartOutcome::Started) => {
            (StatusCode::OK, "started").into_response()
        }
        Ok(crate::detect::control::StartOutcome::AlreadyRunning) => {
            (StatusCode::OK, "already running").into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
    }
}

#[cfg(test)]
#[path = "api_test.rs"]
mod api_test;
