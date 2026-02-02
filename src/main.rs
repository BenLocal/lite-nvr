use std::sync::Arc;

use futures::StreamExt as _;
use tokio_util::sync::CancellationToken;

mod api;
mod media;

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

    // just for testing
    //test_pipe().await;

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

// async fn test_pipe() {
//     let input = PipeInput::Network("http://172.31.169.114:1234/A/video/xin.mp4".to_string());
//     let raw_source = Arc::new(RawSinkSource::new());

//     let raw_source_clone = raw_source.clone();
//     tokio::spawn(async move {
//         let mut stream = RawSinkSource::as_stream(raw_source_clone);
//         while let Some(frame) = stream.next().await {
//             println!("frame: {}", frame.to_string());
//         }
//     });

//     let outputs = vec![
//         PipeOutput::Network("rtsp://172.31.169.114:8554/shiben/3".to_string()),
//         PipeOutput::Raw(raw_source),
//     ];
//     let config = PipeConfig::new(input, outputs);

//     let pipe = new_pipe("test", config).await;
//     tokio::spawn(async move {
//         pipe.start().await;
//     });
// }
