use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use nvr_recorder::{Container, Recorder, RecorderConfig, RtspTransport, TrackSelect};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(about = "Record an RTSP source into time-sliced segment files.")]
struct Args {
    /// RTSP URL, e.g. rtsp://127.0.0.1:8554/stream
    #[arg(long)]
    url: String,
    /// Output directory (created if missing).
    #[arg(long, default_value = "./records")]
    dir: PathBuf,
    /// Segment length in seconds.
    #[arg(long, default_value_t = 60)]
    segment_time: u64,
    /// Tracks to record: video | audio | both.
    #[arg(long, default_value = "both")]
    tracks: String,
    /// Container: ts | mp4 | mkv.
    #[arg(long, default_value = "ts")]
    container: String,
    /// Align segment boundaries to the wall clock (e.g. each minute).
    #[arg(long, default_value_t = false)]
    align: bool,
}

fn parse_tracks(s: &str) -> TrackSelect {
    match s {
        "video" => TrackSelect::Video,
        "audio" => TrackSelect::Audio,
        _ => TrackSelect::Both,
    }
}

fn parse_container(s: &str) -> Container {
    match s {
        "mp4" => Container::Mp4,
        "mkv" => Container::Mkv,
        _ => Container::Ts,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    ffmpeg_bus::init()?;

    let args = Args::parse();
    let mut config = RecorderConfig::new(args.url, &args.dir);
    config.transport = RtspTransport::Tcp;
    config.tracks = parse_tracks(&args.tracks);
    config.container = parse_container(&args.container);
    config.segment_time = Duration::from_secs(args.segment_time);
    config.align_to_wall_clock = args.align;

    std::fs::create_dir_all(&args.dir)?;
    let manifest_path = args.dir.join("manifest.jsonl");
    let mut manifest = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&manifest_path)?;

    let (recorder, mut rx) = Recorder::new(config);
    let cancel = CancellationToken::new();

    let run_cancel = cancel.clone();
    let handle = tokio::spawn(async move { recorder.run(run_cancel).await });

    // Ctrl-C -> graceful stop.
    let sig_cancel = cancel.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        log::info!("interrupt received, stopping recorder");
        sig_cancel.cancel();
    });

    while let Some(info) = rx.recv().await {
        let line = serde_json::to_string(&info)?;
        writeln!(manifest, "{line}")?;
        manifest.flush()?;
        log::info!(
            "segment: {} ({:.3}s, {} bytes)",
            info.path.display(),
            info.duration,
            info.size_bytes
        );
    }

    handle.await??;
    log::info!("recorder stopped; manifest at {}", manifest_path.display());
    Ok(())
}
