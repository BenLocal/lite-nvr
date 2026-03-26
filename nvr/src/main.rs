#[cfg(feature = "zlm")]
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::db::init_app_db;

mod api;
mod config;
mod db;
mod handler;
mod init;
mod manager;
mod media;
#[cfg(feature = "zlm")]
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
    #[cfg(feature = "zlm")]
    {
        // start zlm server
        let cancel_clone = cancel.clone();
        zlm::server::start_zlm_server(cancel_clone, ready_tx).unwrap();
    }
    #[cfg(not(feature = "zlm"))]
    {
        ready_tx.send(()).unwrap();
    }

    // init device pipes
    let cancel_clone = cancel.clone();
    crate::init::device::init_device_pipes(ready_rx, cancel_clone).unwrap();

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
