//! Standalone director bin: decode N sources hot, publish one seamless program
//! stream to ZLM, and switch the active source by typing its id on stdin.
//!
//! ```bash
//! nvr-switcher \
//!   --source camA=rtsp://127.0.0.1/live/a \
//!   --source camB=rtsp://127.0.0.1/live/b \
//!   --publish rtmp://127.0.0.1:8555/switcher/program
//! # then type `camB` <Enter> to cut to B; playback of live/program never breaks.
//! ```

use anyhow::Result;
use clap::Parser;
use tokio::io::AsyncBufReadExt;

use nvr_switcher::{ProgramConfig, Switcher};

#[derive(Parser)]
#[command(
    about = "Seamless GB/RTSP director: switch the program source without interrupting the player"
)]
struct Args {
    /// A source as `id=url`, repeatable. The first source is the initial program.
    #[arg(long = "source", value_parser = parse_source, required = true)]
    sources: Vec<(String, String)>,

    /// Publish URL for the program stream (ZLM).
    #[arg(long, default_value = "rtmp://127.0.0.1:8555/switcher/program")]
    publish: String,

    /// Mux format for the publish URL.
    #[arg(long, default_value = "flv")]
    format: String,

    /// Output frame rate (CFR).
    #[arg(long, default_value_t = 25)]
    fps: u32,

    /// Optional output video bitrate (bps).
    #[arg(long)]
    bitrate: Option<u64>,
}

fn parse_source(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((id, url)) if !id.is_empty() && !url.is_empty() => {
            Ok((id.to_string(), url.to_string()))
        }
        _ => Err(format!("expected `id=url`, got {s:?}")),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    ffmpeg_bus::init()?;

    let args = Args::parse();
    let cfg = ProgramConfig {
        publish_url: args.publish,
        format: args.format,
        fps: args.fps,
        bitrate: args.bitrate,
    };

    let switcher = Switcher::start(args.sources, cfg).await?;
    log::info!(
        "program live. sources: {:?}. Type a source id + Enter to switch; Ctrl-C to quit.",
        switcher.ids()
    );

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::info!("shutting down");
                break;
            }
            line = lines.next_line() => {
                let Some(line) = line? else { break; };
                let id = line.trim();
                if id.is_empty() {
                    continue;
                }
                if switcher.switch(id) {
                    log::info!("switched program -> {id}");
                } else {
                    log::warn!("unknown source id {id:?}; have {:?}", switcher.ids());
                }
            }
        }
    }
    Ok(())
}
