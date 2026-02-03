use tokio_util::sync::CancellationToken;

mod api;
mod manager;
mod media;
mod zlm;

/// 初始化 ez_ffmpeg 详细日志：env_logger (Rust log) + FFmpeg av_log
fn init_ez_ffmpeg_logging() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Trace)
        .filter_module("ez_ffmpeg", log::LevelFilter::Trace)
        .filter_module("ffmpeg_next", log::LevelFilter::Trace)
        .init();
}

#[tokio::main]
async fn main() -> ! {
    init_ez_ffmpeg_logging();
    let cancel = CancellationToken::new();

    let cancel_clone = cancel.clone();
    api::start_api_server(cancel_clone);
    let cancel_clone = cancel.clone();
    zlm::start_zlm_server(cancel_clone);

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
