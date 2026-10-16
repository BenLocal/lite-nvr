//! `/api/llm` routes: OpenAI-compatible chat completions, the model list and
//! a health probe. Authenticated like the rest of `/api`; OpenAI clients pass
//! the session token as their API key (`Authorization: Bearer`).

use std::convert::Infallible;
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures::{StreamExt, stream};
use nvr_rkllm::{Delta, Engine, EngineState, GenerateError};
use serde_json::{Value, json};

use super::Llm;
use super::openai::{Completion, CompletionRequest, error_body};

pub fn llm_router() -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat))
        .route("/v1/models", get(models))
        .route("/health", get(health))
}

fn openai_error(status: StatusCode, message: &str, kind: &str) -> Response {
    (status, Json(error_body(message, kind))).into_response()
}

fn unavailable(message: &str) -> Response {
    openai_error(
        StatusCode::SERVICE_UNAVAILABLE,
        message,
        "service_unavailable",
    )
}

/// The engine and model id, or why the LLM is unavailable.
fn enabled(llm: &'static Llm) -> Result<(&'static Engine, &'static str), String> {
    match llm {
        Llm::Enabled { engine, model } => Ok((engine, model)),
        Llm::Disabled => Err("LLM is disabled (NVR_LLM_MODEL is not set)".into()),
        Llm::Invalid(reason) => Err(format!("LLM is misconfigured: {reason}")),
    }
}

async fn chat(body: Result<Json<CompletionRequest>, JsonRejection>) -> Response {
    let Json(req) = match body {
        Ok(body) => body,
        Err(err) => {
            return openai_error(
                StatusCode::BAD_REQUEST,
                &err.body_text(),
                "invalid_request_error",
            );
        }
    };
    let (engine, model) = match enabled(super::get()) {
        Ok(found) => found,
        Err(reason) => return unavailable(&reason),
    };
    let streaming = req.stream;
    let include_usage = req.include_usage();
    let chat = match req.into_chat() {
        Ok(chat) => chat,
        Err(message) => {
            return openai_error(StatusCode::BAD_REQUEST, &message, "invalid_request_error");
        }
    };
    let generation = match engine.generate(chat) {
        Ok(generation) => generation,
        Err(GenerateError::Busy) => {
            return openai_error(
                StatusCode::TOO_MANY_REQUESTS,
                "model is busy, retry later",
                "rate_limit_exceeded",
            );
        }
        Err(err) => return unavailable(&err.to_string()),
    };
    let completion = Arc::new(Completion::new(model));

    // Dropping the response (client gone) drops the generation, which stops it.
    if !streaming {
        let deltas: Vec<Delta> = generation.collect().await;
        return match completion.response(deltas) {
            Ok(body) => Json(body).into_response(),
            Err(message) => {
                openai_error(StatusCode::INTERNAL_SERVER_ERROR, &message, "server_error")
            }
        };
    }
    let data = |value: Value| Event::default().data(value.to_string());
    let head = stream::iter([data(completion.role_chunk())]);
    let body = generation.flat_map(move |delta| {
        stream::iter(
            completion
                .chunks(delta, include_usage)
                .into_iter()
                .map(data)
                .collect::<Vec<_>>(),
        )
    });
    let tail = stream::iter([Event::default().data("[DONE]")]);
    let events = head.chain(body).chain(tail).map(Ok::<_, Infallible>);
    Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn models() -> Json<Value> {
    let data = match super::get() {
        Llm::Enabled { model, .. } => {
            vec![json!({ "id": model, "object": "model", "created": 0, "owned_by": "nvr" })]
        }
        _ => Vec::new(),
    };
    Json(json!({ "object": "list", "data": data }))
}

async fn health() -> Response {
    let (status, body) = health_body(super::get());
    (status, Json(body)).into_response()
}

/// 200 only when the model is ready; 503 with the reason otherwise.
pub(crate) fn health_body(llm: &Llm) -> (StatusCode, Value) {
    let unavailable = StatusCode::SERVICE_UNAVAILABLE;
    match llm {
        Llm::Disabled => (unavailable, json!({ "status": "disabled" })),
        Llm::Invalid(reason) => (unavailable, json!({ "status": "failed", "reason": reason })),
        Llm::Enabled { engine, model } => match engine.state() {
            EngineState::Ready => (StatusCode::OK, json!({ "status": "ready", "model": model })),
            EngineState::Loading => (unavailable, json!({ "status": "loading", "model": model })),
            EngineState::Failed(reason) => (
                unavailable,
                json!({ "status": "failed", "model": model, "reason": reason }),
            ),
        },
    }
}

#[cfg(test)]
#[path = "api_test.rs"]
mod api_test;
