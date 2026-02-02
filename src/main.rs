use std::sync::Arc;

use futures::StreamExt as _;
use jpeg_encoder::{ColorType, Encoder};
use tokio_util::sync::CancellationToken;

use crate::media::{pipe::Pipe, stream::RawSinkSource, types::PipeConfig};

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
    test_pipe().await;

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

async fn test_pipe() {
    let raw_source = Arc::new(RawSinkSource::new());
    let config = PipeConfig::builder()
        .input_file("scripts/test.mp4")
        .add_raw_frame_output(raw_source.clone())
        .build();

    let raw_source_clone = raw_source.clone();
    tokio::spawn(async move {
        let mut stream = RawSinkSource::as_stream(raw_source_clone);
        let mut frame_count = 0u32;
        while let Some(frame) = stream.next().await {
            println!(
                "frame: {} ({}x{})",
                frame.to_string(),
                frame.width,
                frame.height
            );

            // Convert YUV420P to RGB and save as JPEG
            let rgb_data = frame.to_rgb();
            let filename = format!("frame_{:04}.jpg", frame_count);
            let encoder = Encoder::new_file(&filename, 90).unwrap();
            encoder
                .encode(
                    &rgb_data,
                    frame.width as u16,
                    frame.height as u16,
                    ColorType::Rgb,
                )
                .unwrap();

            println!("Saved {}", filename);
            frame_count += 1;
            if frame_count >= 10 {
                println!("Saved {} frames, stopping...", frame_count);
                break;
            }
        }
    });

    let pipe = Pipe::new(config);
    tokio::spawn(async move {
        pipe.start().await;
    });
}
