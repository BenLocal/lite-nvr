use axum::{
    extract::Path,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use crate::{
    manager,
    media::types::{InputConfig, OutputConfig, OutputDest, PipeConfig},
};

pub(crate) fn start_api_server(cancel: CancellationToken) {
    tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(index))
            .route("/pipe/list", get(list_pipes))
            .route("/pipe/add", post(add_pipe))
            .route("/pipe/remove/{id}", get(remove_pipe));

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
    url: String,
    format: String,
}

async fn add_pipe(Json(config): Json<PipeRequest>) -> Json<String> {
    let pipe_config = PipeConfig {
        input: InputConfig::Network {
            url: config.input.url,
        },
        outputs: config
            .outputs
            .into_iter()
            .map(|output| OutputConfig {
                dest: OutputDest::Network {
                    url: output.url,
                    format: output.format,
                },
                encode: None,
            })
            .collect(),
    };
    if let Err(e) = manager::add_pipe(&config.id, pipe_config, false).await {
        return Json(e.to_string());
    }
    Json("success".to_string())
}

async fn remove_pipe(Path(id): Path<String>) -> Json<String> {
    if let Err(e) = manager::remove_pipe(&id).await {
        return Json(e.to_string());
    }
    Json("success".to_string())
}
