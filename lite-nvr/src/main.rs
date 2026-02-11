use tokio_util::sync::CancellationToken;

mod api;
mod config;
mod handler;
mod manager;
mod media;
#[cfg(feature = "zlm")]
mod zlm;

fn init_logging() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Trace)
        .filter_module("ffmpeg_next", log::LevelFilter::Trace)
        .filter_module("ffmpeg_bus", log::LevelFilter::Trace)
        .init();
}

#[tokio::main]
async fn main() -> ! {
    init_logging();
    ffmpeg_bus::init().expect("ffmpeg_bus init");
    let config = config::config();
    nvr_db::migrations::migrate(config.db_url())
        .await
        .unwrap_or_else(|e| {
            eprintln!("Error migrating database: {}", e);
            std::process::exit(1);
        });

    let cancel = CancellationToken::new();

    let cancel_clone = cancel.clone();
    api::start_api_server(cancel_clone);

    #[cfg(feature = "zlm")]
    {
        let cancel_clone = cancel.clone();
        zlm::start_zlm_server(cancel_clone);
    }

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
