//! Detection control + read endpoints. Opt-in start/stop per pipe; GET latest
//! per-frame multi-model result. GET/POST only; session auth is applied by the
//! parent `/api` router.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use super::control::{
    DETECT_SUPPORTED_INPUT_TYPES, MAX_DETECT_MODEL_NAME_CHARS, MAX_DETECT_MODELS,
    MAX_DETECT_SAMPLE_INTERVAL_MS, normalize_model_names,
};
use super::hub::{DetectHub, TapEpoch};

const TAP_LEASE_HEADER: &str = "x-detection-tap-lease";

#[derive(Deserialize, Default)]
pub struct StartBody {
    /// Subset of configured model names to run. Absent/empty = all.
    #[serde(default)]
    pub models: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct StopBody {
    #[serde(default)]
    lease: Option<String>,
}

#[derive(Serialize)]
struct DetectionCapabilities {
    models: Vec<String>,
    supported_input_types: &'static [&'static str],
    max_sample_interval_ms: u64,
    max_model_count: usize,
    max_model_name_chars: usize,
}

pub fn detect_router(hub: &'static DetectHub) -> Router {
    Router::new()
        .route("/{pipe}/start", post(start))
        .route("/{pipe}/stop", post(stop))
        .route("/{pipe}/latest", get(latest))
        .route("/models", get(models))
        .route("/capabilities", get(capabilities))
        .with_state(hub)
}

async fn capabilities(State(hub): State<&'static DetectHub>) -> Json<DetectionCapabilities> {
    Json(DetectionCapabilities {
        models: hub.config_names(),
        supported_input_types: DETECT_SUPPORTED_INPUT_TYPES,
        max_sample_interval_ms: MAX_DETECT_SAMPLE_INTERVAL_MS,
        max_model_count: MAX_DETECT_MODELS,
        max_model_name_chars: MAX_DETECT_MODEL_NAME_CHARS,
    })
}

async fn models(State(hub): State<&'static DetectHub>) -> impl IntoResponse {
    Json(hub.config_names()).into_response()
}

async fn latest(
    State(hub): State<&'static DetectHub>,
    Path(pipe): Path<String>,
) -> impl IntoResponse {
    match hub.latest(&pipe) {
        Some(fr) => Json(fr).into_response(),
        None => (StatusCode::NOT_FOUND, "no result yet").into_response(),
    }
}

async fn stop(
    State(hub): State<&'static DetectHub>,
    Path(pipe): Path<String>,
    body: Option<Json<StopBody>>,
) -> impl IntoResponse {
    let stopped = match body.and_then(|Json(body)| body.lease) {
        Some(lease) => match lease.parse::<u64>() {
            Ok(raw) => hub.stop_tap(&pipe, TapEpoch::from_raw(raw)),
            Err(_) => return (StatusCode::BAD_REQUEST, "invalid tap lease").into_response(),
        },
        None => hub.stop(&pipe),
    };
    if stopped {
        (StatusCode::OK, "stopped").into_response()
    } else {
        (StatusCode::OK, "not running").into_response()
    }
}

async fn start(
    State(hub): State<&'static DetectHub>,
    Path(pipe): Path<String>,
    body: Option<Json<StartBody>>,
) -> impl IntoResponse {
    let want = body.and_then(|Json(b)| b.models);
    let want = match want {
        Some(models) => match normalize_model_names(&models) {
            Ok(models) if models.is_empty() => None,
            Ok(models) => Some(models),
            Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        },
        None => None,
    };
    match crate::detect::control::start_tap(hub, &pipe, want, 0, 0.0, None).await {
        Ok(crate::detect::control::StartOutcome::Started(epoch)) => {
            let mut response = (StatusCode::OK, "started").into_response();
            response.headers_mut().insert(
                TAP_LEASE_HEADER,
                HeaderValue::from_str(&epoch.raw().to_string()).expect("u64 is a valid header"),
            );
            response
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
