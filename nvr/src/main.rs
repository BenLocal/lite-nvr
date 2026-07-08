use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::db::init_app_db;

mod api;
mod asr;
mod audiomixer;
mod cleanup;
mod compositor;
mod config;
mod db;
mod gb;
mod handler;
mod init;
mod manager;
mod metrics;
mod program;
mod proxy;
mod transport;
mod xiaomi;
mod zlm;

fn init_logging() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        //.filter_module("ffmpeg_next", log::LevelFilter::Trace)
        //.filter_module("ffmpeg_bus", log::LevelFilter::Trace)
        .init();
}

#[tokio::main]
async fn main() -> ! {
    init_logging();
    ffmpeg_bus::init().expect("ffmpeg_bus init");

    // migrate database
    let config = config::config();
    nvr_db::migrations::migrate(config.db_url())
        .await
        .unwrap_or_else(|e| {
            log::error!("Error migrating database: {}", e);
            std::process::exit(1);
        });

    // init app db
    init_app_db(config.db_url()).await.unwrap();
    nvr_db::migrations::ensure_default_admin_user(config.db_url())
        .await
        .unwrap_or_else(|e| {
            log::error!("Error ensuring default admin user: {}", e);
            std::process::exit(1);
        });

    let cancel = CancellationToken::new();

    let (ready_tx, ready_rx) = oneshot::channel();
    // start zlm server
    let cancel_clone = cancel.clone();
    zlm::server::start_zlm_server(cancel_clone, ready_tx).unwrap();

    // start the GB28181 platform (on-demand bridge) if configured
    if let Some(gb_cfg) = config.gb().cloned() {
        if let Err(e) = crate::gb::init(gb_cfg).await {
            log::error!("Failed to init gb28181 bridge: {:#}", e);
        }
    }

    // init device pipes
    let cancel_clone = cancel.clone();
    crate::init::device::init_device_pipes(ready_rx, cancel_clone).unwrap();

    // start the record-segment transport worker (copies segments to remote
    // storage targets configured via the API)
    transport::spawn_worker(cancel.clone());

    // start the record-segment retention cleanup worker (deletes old segments
    // per the policy configured on the dashboard Settings page)
    cleanup::spawn_worker(cancel.clone());

    // start the system-metrics sampler (CPU / memory / network into a cache the
    // dashboard homepage polls)
    metrics::spawn_worker(cancel.clone());

    // start api server
    let cancel_clone = cancel.clone();
    api::start_api_server(cancel_clone, 18080);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            },
            _ = tokio::signal::ctrl_c() => {
                cancel.cancel();
            },
        }
    }

    std::process::exit(0);
}
