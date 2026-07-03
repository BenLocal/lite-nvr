//! Standalone multi-view compositor bin: decode N sources, composite them into
//! one stream (grid by default, or free-form `--region`s), publish to ZLM.
//!
//! ```bash
//! nvr-compositor \
//!   --source a=rtsp://.../a --source b=rtsp://.../b \
//!   --source c=rtsp://.../c --source d=rtsp://.../d \
//!   --canvas 1280x720 \
//!   --publish rtmp://127.0.0.1:8555/live/mosaic
//! # 4 sources -> 2x2 grid. Play http://127.0.0.1:8553/live/mosaic.live.flv
//!
//! # picture-in-picture via explicit regions:
//! nvr-compositor --source main=rtsp://.../m --source pip=rtsp://.../p \
//!   --canvas 1280x720 \
//!   --region main=0,0,1280,720 --region pip=960,540,320,180
//! ```
//!
//! While running, type `<region-index> <source-id>` on stdin to switch any
//! region to any source in the pool live (e.g. `0 c`).

use anyhow::Result;
use clap::Parser;

use nvr_compositor::{Compositor, CompositorConfig, Layout, Region, Source, SourceFeed};

#[derive(Parser)]
#[command(about = "Multi-view compositor: fuse several sources into one ZLM stream")]
struct Args {
    /// A source as `id=url`, repeatable.
    #[arg(long = "source", value_parser = parse_source, required = true)]
    sources: Vec<(String, String)>,

    /// Output canvas size, `WxH`.
    #[arg(long, default_value = "1280x720", value_parser = parse_size)]
    canvas: (u32, u32),

    /// A region as `id=x,y,w,h`, repeatable. If none are given, sources are laid
    /// out in an automatic grid.
    #[arg(long = "region", value_parser = parse_region)]
    regions: Vec<Region>,

    /// Publish URL for the composited stream (ZLM).
    #[arg(long, default_value = "rtmp://127.0.0.1:8555/live/mosaic")]
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

fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("expected `WxH`, got {s:?}"))?;
    let w = w
        .trim()
        .parse()
        .map_err(|_| format!("bad width in {s:?}"))?;
    let h = h
        .trim()
        .parse()
        .map_err(|_| format!("bad height in {s:?}"))?;
    Ok((w, h))
}

fn parse_region(s: &str) -> Result<Region, String> {
    let (id, rect) = s
        .split_once('=')
        .ok_or_else(|| format!("expected `id=x,y,w,h`, got {s:?}"))?;
    let nums: Vec<u32> = rect
        .split(',')
        .map(|p| p.trim().parse::<u32>())
        .collect::<Result<_, _>>()
        .map_err(|_| format!("bad numbers in region {s:?}"))?;
    match nums.as_slice() {
        [x, y, w, h] if !id.is_empty() => Ok(Region {
            source_id: id.to_string(),
            x: *x,
            y: *y,
            w: *w,
            h: *h,
        }),
        _ => Err(format!("expected `id=x,y,w,h`, got {s:?}")),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    ffmpeg_bus::init()?;

    let args = Args::parse();

    let mut sources = Vec::with_capacity(args.sources.len());
    for (id, url) in &args.sources {
        sources.push(Source::start(id, url).await?);
    }
    let template = sources[0].video_stream.clone();
    let feeds: Vec<SourceFeed> = sources
        .iter()
        .map(|s| SourceFeed {
            id: s.id.clone(),
            latest: s.latest.clone(),
        })
        .collect();

    let (cw, ch) = args.canvas;
    let layout = if args.regions.is_empty() {
        let ids: Vec<String> = args.sources.iter().map(|(id, _)| id.clone()).collect();
        Layout::grid(cw, ch, &ids)
    } else {
        Layout::new(cw, ch, args.regions)
    };

    let cfg = CompositorConfig {
        publish_url: args.publish,
        format: args.format,
        fps: args.fps,
        bitrate: args.bitrate,
    };
    let compositor = Compositor::start(cfg, layout, feeds, template);
    log::info!(
        "compositor live ({} regions). Type `<region> <source-id>` to switch, Ctrl-C to quit.",
        compositor.region_count()
    );

    // Read stdin lines and switch regions live.
    let director = compositor.director();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            let mut it = line.split_whitespace();
            match (it.next().and_then(|s| s.parse::<usize>().ok()), it.next()) {
                (Some(region), Some(source)) => {
                    if director.switch(region, source) {
                        log::info!("switched region {region} -> {source}");
                    } else {
                        log::warn!("switch rejected: region {region} / source {source:?}");
                    }
                }
                _ => log::warn!("usage: <region-index> <source-id>"),
            }
        }
    });

    tokio::signal::ctrl_c().await?;
    log::info!("shutting down");
    compositor.stop();
    let _ = compositor.join().await;
    drop(sources); // stop decoding
    Ok(())
}
