//! ASR control endpoints: opt-in start/stop per pipe. GET/POST only.

use axum::{Router, extract::Path, http::StatusCode, response::IntoResponse, routing::post};
use tokio_util::sync::CancellationToken;

use super::hub::AsrHub;

pub fn asr_router() -> Router {
    Router::new()
        .route("/{pipe}/start", post(start))
        .route("/{pipe}/stop", post(stop))
}

async fn start(Path(pipe): Path<String>) -> impl IntoResponse {
    let Some(hub) = AsrHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "asr not initialized").into_response();
    };
    if hub.is_running(&pipe) {
        return (StatusCode::OK, "already running").into_response();
    }
    let Some(handle) = crate::manager::get_pipe(&pipe).await else {
        return (StatusCode::NOT_FOUND, "pipe not found").into_response();
    };
    let audio = match handle.subscribe_audio().await {
        Ok(rx) => rx,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("no audio: {e:#}")).into_response(),
    };
    let models = match hub.models().await {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("model load failed (run `make download-asr-models`?): {e:#}"),
            )
                .into_response();
        }
    };
    let cancel = CancellationToken::new();
    if !hub.register(&pipe, cancel.clone()) {
        return (StatusCode::OK, "already running").into_response();
    }
    let io = hub.io().clone();
    // The tap holds a non-`Send` ffmpeg resampler across await points, so it
    // can't ride the multi-threaded runtime via `tokio::spawn`. All the inputs
    // are `Send`, so run it on a dedicated thread with its own current-thread
    // runtime; `cancel` (fired by `hub.unregister`) unwinds it.
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                log::error!("asr[{pipe}]: failed to build tap runtime: {e:#}");
                return;
            }
        };
        rt.block_on(super::tap::run(pipe, models, audio, io, cancel));
    });
    (StatusCode::OK, "started").into_response()
}

async fn stop(Path(pipe): Path<String>) -> impl IntoResponse {
    let Some(hub) = AsrHub::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "asr not initialized").into_response();
    };
    if hub.unregister(&pipe) {
        (StatusCode::OK, "stopped").into_response()
    } else {
        (StatusCode::OK, "not running").into_response()
    }
}
