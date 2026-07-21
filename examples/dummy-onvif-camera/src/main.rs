mod auth;
mod config;
mod discovery;
mod responses;
mod soap;

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use clap::Parser;

use crate::config::DeviceCfg;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let (cfg, opts) = config::Args::parse().into_cfg();
    let cfg = Arc::new(cfg);

    // Optional: spawn dummy-rtsp-camera so the RTSP url actually serves.
    let mut _rtsp_child = None;
    if opts.launch_rtsp {
        match which::which("cargo") {
            Ok(cargo) => {
                log::info!("launching dummy-rtsp-camera …");
                _rtsp_child = Some(
                    std::process::Command::new(cargo)
                        .args(["run", "-q", "-p", "dummy-rtsp-camera"])
                        .spawn()?,
                );
            }
            Err(_) => log::warn!("--launch-rtsp: cargo not found; run dummy-rtsp-camera yourself"),
        }
    } else {
        log::info!(
            "GetStreamUri will return {} — run dummy-rtsp-camera to serve it",
            cfg.rtsp_url
        );
    }

    // WS-Discovery responder.
    if opts.discovery {
        let dcfg = (*cfg).clone();
        tokio::spawn(async move {
            if let Err(e) = discovery::run(dcfg).await {
                log::error!("ws-discovery stopped: {e}");
            }
        });
    }

    // SOAP HTTP server.
    let app = Router::new()
        .route("/onvif/device_service", post(soap_handler))
        .with_state(cfg.clone());
    let addr = format!("0.0.0.0:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    log::info!(
        "dummy-onvif-camera: SOAP at {} (user {})",
        cfg.service_url(),
        cfg.username
    );
    axum::serve(listener, app).await?;
    Ok(())
}

async fn soap_handler(State(cfg): State<Arc<DeviceCfg>>, body: Bytes) -> Response {
    let text = String::from_utf8_lossy(&body);
    let reply = soap::handle(&text, &cfg);
    Response::builder()
        .status(StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header("Content-Type", "application/soap+xml; charset=utf-8")
        .body(reply.body.into())
        .expect("valid response")
}
