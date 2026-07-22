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
use tokio_util::sync::CancellationToken;

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
    if hub.is_running(&pipe) {
        return (StatusCode::OK, "already running").into_response();
    }
    let want = body.and_then(|Json(b)| b.models);

    let Some(handle) = crate::manager::get_pipe(&pipe).await else {
        return (StatusCode::NOT_FOUND, "pipe not found").into_response();
    };
    let video = match handle.subscribe_video().await {
        Ok(rx) => rx,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("no video: {e:#}")).into_response(),
    };

    let all = match hub.detectors().await {
        Ok(d) => d,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("model load failed: {e:#}"),
            )
                .into_response();
        }
    };
    let detectors = hub.detectors_named(&all, &want);
    if detectors.is_empty() {
        return (StatusCode::BAD_REQUEST, "no matching models").into_response();
    }

    let cancel = CancellationToken::new();
    if !hub.register(&pipe, cancel.clone()) {
        return (StatusCode::OK, "already running").into_response();
    }
    let interval = hub.sample_interval_ms();
    tokio::spawn(super::tap::run(
        pipe, detectors, video, interval, hub, cancel,
    ));
    (StatusCode::OK, "started").into_response()
}

#[cfg(test)]
#[path = "api_test.rs"]
mod api_test;
