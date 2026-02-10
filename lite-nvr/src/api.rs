use std::sync::Arc;

use axum::{
    extract::Path,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::{
    manager,
    media::types::{InputConfig, OutputConfig, OutputDest, PipeConfig},
};

pub type ApiResult<T> = Result<T, ApiError>;
pub type ApiJsonResult<T> = ApiResult<Json<T>>;

pub struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        eprintln!("ApiError: {:?}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Manager went wrong because service inner error"),
        )
            .into_response()
    }
}

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

pub(crate) fn start_api_server(cancel: CancellationToken) {
    tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(index))
            .route("/pipe/list", get(list_pipes))
            .route("/pipe/add", post(add_pipe))
            .route("/pipe/remove/{id}", get(remove_pipe))
            .route("/pipe/status/{id}", get(get_pipe_status));

        let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
        println!("API server started on port 8080");
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal(cancel))
            .await
        {
            println!("Error starting API server: {}", e);
        }
    });
}

async fn shutdown_signal(cancel: CancellationToken) {
    tokio::select! {
        _ = cancel.cancelled() => {
            println!("Shutting down API server...");
        }
    }
}

async fn index() -> &'static str {
    "Hello, world!"
}

async fn list_pipes() -> Json<Vec<String>> {
    let pipes = manager::get_pipe_manager().read().await;
    Json(pipes.keys().cloned().collect())
}

#[derive(Serialize, Deserialize)]
struct PipeRequest {
    id: String,
    input: InputRequest,
    outputs: Vec<OutputRequest>,
}

#[derive(Serialize, Deserialize)]
struct InputRequest {
    url: String,
}

#[derive(Serialize, Deserialize)]
struct OutputRequest {
    net: Option<NetConfigRequest>,
    t: Option<String>,
    zlm: Option<ZlmConfigRequest>,
}

#[derive(Serialize, Deserialize)]
struct ZlmConfigRequest {
    app: String,
    stream: String,
}

#[derive(Serialize, Deserialize)]
struct NetConfigRequest {
    url: String,
    format: String,
}

async fn add_pipe(Json(config): Json<PipeRequest>) -> ApiJsonResult<String> {
    let mut outputs = Vec::new();
    for output in config.outputs {
        let dest = match output.t.unwrap_or_default().as_str() {
            "zlm" => {
                if let Some(zlm) = output.zlm {
                    OutputDest::Zlm(Arc::new(rszlm::media::Media::new(
                        "__defaultVhost__",
                        zlm.app.as_str(),
                        zlm.stream.as_str(),
                        0.0,
                        false,
                        false,
                    )))
                } else {
                    return Err(anyhow::anyhow!("zlm config is required").into());
                }
            }
            _ => {
                if let Some(net) = output.net {
                    OutputDest::Network {
                        url: net.url,
                        format: net.format,
                    }
                } else {
                    return Err(anyhow::anyhow!("net config is required").into());
                }
            }
        };
        outputs.push(OutputConfig { dest, encode: None });
    }

    if outputs.is_empty() {
        return Err(anyhow::anyhow!("outputs is required").into());
    }

    let pipe_config = PipeConfig {
        input: InputConfig::Network {
            url: config.input.url,
        },
        outputs: outputs,
    };
    manager::add_pipe(&config.id, pipe_config, false).await?;
    Ok(Json("success".to_string()))
}

async fn remove_pipe(Path(id): Path<String>) -> ApiJsonResult<String> {
    manager::remove_pipe(&id).await?;
    Ok(Json("success".to_string()))
}

async fn get_pipe_status(Path(id): Path<String>) -> ApiJsonResult<String> {
    let pipe = manager::get_pipe(&id).await;
    if let Some(pipe) = pipe {
        return Ok(Json(pipe.is_started().to_string()));
    }
    Ok(Json("not found".to_string()))
}
