use std::sync::Arc;

use axum::{
    Json, Router,
    extract::Path,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::{
    handler::ApiJsonResult,
    manager,
    media::types::{InputConfig, OutputConfig, OutputDest, PipeConfig},
};

pub fn meida_pipe_router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/list", get(list_pipes))
        .route("/add", post(add_pipe))
        .route("/remove/{id}", get(remove_pipe))
        .route("/status/{id}", get(get_pipe_status))
}

#[derive(Serialize, Deserialize)]
struct PipeRequest {
    id: String,
    input: InputRequest,
    outputs: Vec<OutputRequest>,
}

#[derive(Serialize, Deserialize)]
struct InputRequest {
    t: String,
    input: String,
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

async fn index() -> &'static str {
    "pipe route!"
}

async fn list_pipes() -> Json<Vec<String>> {
    let pipes = manager::get_pipe_manager().read().await;
    Json(pipes.keys().cloned().collect())
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

    let input = match config.input.t.as_ref() {
        "net" => InputConfig::Network {
            url: config.input.input,
        },
        "file" => InputConfig::File {
            path: config.input.input,
        },
        "v4l2" => InputConfig::V4L2 {
            device: config.input.input,
        },
        "x11grab" => InputConfig::X11Grab {
            display: config.input.input,
        },
        _ => return Err(anyhow::anyhow!("input type is not supported").into()),
    };

    let pipe_config = PipeConfig {
        input: input,
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
