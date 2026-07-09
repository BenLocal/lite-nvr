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
mod livestream;
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
        // Drop the noisy span-enter INFO records that libsql/turso emit through
        // tracing's log bridge (target `tracing::span`: _prepare, consume_stmt,
        // _connect, connect_with_encryption, …); keep any real warnings/errors.
        .filter_module("tracing", log::LevelFilter::Warn)
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

    // Graceful shutdown. `std::process::exit` runs ZLM/ffmpeg C static
    // destructors immediately; if a media thread is still writing into ZLM at
    // that moment it touches freed memory and segfaults. So first stop every
    // producer that feeds ffmpeg/ZLM (director programs, compositor programs,
    // mixer buses, GB pulls, device pipes) and let their threads unwind, THEN
    // exit. None of these clear their persisted config, so everything restores
    // on the next start. A timeout guards against a stuck teardown hanging the
    // exit. Stop the ZLM-writing producers (program/compositor/mixer) before the
    // device pipes; GB is best-effort.
    log::info!("shutting down: stopping media producers…");
    let teardown = async {
        crate::program::shutdown().await;
        crate::compositor::shutdown().await;
        crate::audiomixer::shutdown();
        crate::gb::shutdown().await;
        crate::manager::shutdown().await;
        // With every producer stopped, tear ZLM's servers/sessions down while
        // the process is still fully alive. Leaving live sessions (external
        // RTSP pushers, players) to exit-time C++ static destruction is what
        // kept segfaulting after the producer-side fixes.
        let _ = tokio::task::spawn_blocking(crate::zlm::server::stop_all).await;
    };
    if tokio::time::timeout(std::time::Duration::from_secs(5), teardown)
        .await
        .is_err()
    {
        log::warn!("shutdown: media teardown timed out; exiting anyway");
    }
    // Brief settle for any in-flight ZLM writes/callbacks to drain before the
    // C runtime is torn down.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    log::info!("shutdown complete");

    std::process::exit(0);
}
