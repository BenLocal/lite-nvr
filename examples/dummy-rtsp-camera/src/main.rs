//! A dummy RTSP camera: a thin launcher around the external
//! [`oddity-rtsp-server`](https://crates.io/crates/oddity-rtsp-server)
//! binary — no hand-rolled RTSP/RTP/H.264 handling here.
//!
//! It generates a short H.264 test clip with ffmpeg (once, cached), writes
//! the YAML config oddity-rtsp-server expects, and spawns the server. File
//! sources loop forever in oddity-rtsp-server, so the mount behaves like an
//! always-on camera. Audio is not served (oddity-rtsp-server is video-only).
//!
//! ```bash
//! # once — run from the repo root: .cargo/config.toml already points
//! # FFMPEG_DIR at the bundled FFmpeg 7.1, which video-rs builds against
//! cargo install oddity-rtsp-server
//!
//! cargo run -p dummy-rtsp-camera
//! ffplay -rtsp_transport tcp rtsp://127.0.0.1:9554/live/test1
//! ```
//!
//! The server speaks RTP over TCP (interleaved) only; players that try UDP
//! first log a single "461 Unsupported Transport" and then succeed over TCP.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "dummy-rtsp-camera")]
struct Args {
    /// RTSP listen address
    #[arg(long, default_value = "0.0.0.0")]
    listen: String,

    /// RTSP listen port
    #[arg(long, default_value_t = 9554)]
    port: u16,

    /// Stream path (e.g. /live/test1)
    #[arg(long, default_value = "/live/test1")]
    path: String,

    /// Video width of the generated test clip
    #[arg(long, default_value_t = 1920)]
    width: u32,

    /// Video height of the generated test clip
    #[arg(long, default_value_t = 1080)]
    height: u32,

    /// Frame rate of the generated test clip
    #[arg(long, default_value_t = 25)]
    fps: u32,

    /// x264 preset used when generating the test clip
    #[arg(long, default_value = "ultrafast")]
    preset: String,

    /// ffmpeg binary used to generate the test clip.
    /// Default: $FFMPEG_DIR/bin/ffmpeg if it runs, else `ffmpeg` from PATH.
    #[arg(long)]
    ffmpeg: Option<String>,

    /// Serve this media file instead of generating a test clip
    #[arg(long)]
    media: Option<PathBuf>,

    /// oddity-rtsp-server binary.
    /// Defaults to $ODDITY_RTSP_SERVER_BIN, else `oddity-rtsp-server` on PATH.
    #[arg(long)]
    server_bin: Option<String>,
}

/// An ffmpeg invocation target; bundled builds also carry the lib dir their
/// binary needs on LD_LIBRARY_PATH (without it they fail with exit 127,
/// "error while loading shared libraries").
struct Ffmpeg {
    bin: PathBuf,
    lib_dir: Option<PathBuf>,
}

impl Ffmpeg {
    fn command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.bin);
        if let Some(lib) = &self.lib_dir {
            cmd.env(
                "LD_LIBRARY_PATH",
                prepend_ld_path(lib, std::env::var_os("LD_LIBRARY_PATH")),
            );
        }
        cmd
    }

    /// Cheap probe: does this candidate actually execute?
    async fn runs(&self) -> bool {
        self.command()
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// The bundled FFmpeg lib dir, when present. Both the bundled ffmpeg binary
/// and an oddity-rtsp-server installed from the repo root (.cargo/config.toml
/// sets FFMPEG_DIR there) link against these shared libraries, so child
/// processes need the dir on LD_LIBRARY_PATH or they die with exit 127.
fn ffmpeg_lib_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("FFMPEG_DIR")?).join("lib");
    dir.is_dir().then_some(dir)
}

fn prepend_ld_path(
    lib: &std::path::Path,
    existing: Option<std::ffi::OsString>,
) -> std::ffi::OsString {
    let mut value = std::ffi::OsString::from(lib);
    if let Some(existing) = existing
        && !existing.is_empty()
    {
        value.push(":");
        value.push(existing);
    }
    value
}

/// Pick an ffmpeg that actually runs. An explicit --ffmpeg is honored as-is;
/// otherwise the bundled $FFMPEG_DIR/bin/ffmpeg is tried (with its own lib
/// dir), and when that can't execute the fallback is `ffmpeg` located on
/// PATH via `which`.
async fn resolve_ffmpeg(args: &Args) -> anyhow::Result<Ffmpeg> {
    if let Some(bin) = &args.ffmpeg {
        return Ok(Ffmpeg {
            bin: PathBuf::from(bin),
            lib_dir: None,
        });
    }

    let mut tried = Vec::new();
    if let Ok(dir) = std::env::var("FFMPEG_DIR") {
        let dir = PathBuf::from(dir);
        let candidate = Ffmpeg {
            bin: dir.join("bin").join("ffmpeg"),
            lib_dir: Some(dir.join("lib")),
        };
        if candidate.bin.is_file() {
            if candidate.runs().await {
                return Ok(candidate);
            }
            tried.push(format!(
                "{} (present but does not run)",
                candidate.bin.display()
            ));
        }
    }

    match which::which("ffmpeg") {
        Ok(bin) => {
            let candidate = Ffmpeg { bin, lib_dir: None };
            if candidate.runs().await {
                return Ok(candidate);
            }
            tried.push(format!(
                "{} (present but does not run)",
                candidate.bin.display()
            ));
        }
        Err(_) => tried.push("`ffmpeg` on PATH (not found)".to_string()),
    }

    anyhow::bail!(
        "no working ffmpeg found; tried: {}. Pass --ffmpeg <path> to pick one explicitly.",
        tried.join(", ")
    )
}

fn resolve_server_bin(args: &Args) -> String {
    args.server_bin.clone().unwrap_or_else(|| {
        std::env::var("ODDITY_RTSP_SERVER_BIN").unwrap_or_else(|_| "oddity-rtsp-server".to_string())
    })
}

/// Mount paths must start with `/` for the URL in the config to be valid.
fn normalize_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// Cache file name for a generated clip; parameters are baked into the name
/// so changing them regenerates instead of serving a stale clip.
fn clip_cache_path(args: &Args) -> PathBuf {
    std::env::temp_dir().join(format!(
        "dummy-rtsp-camera-{}x{}-{}fps-{}.mp4",
        args.width, args.height, args.fps, args.preset
    ))
}

fn render_config(args: &Args, media: &std::path::Path) -> String {
    format!(
        "server:\n  host: \"{}\"\n  port: {}\nmedia:\n  - name: \"dummy camera\"\n    path: \"{}\"\n    kind: file\n    source: \"{}\"\n",
        args.listen,
        args.port,
        normalize_path(&args.path),
        media.display()
    )
}

/// Generate the looping test clip if it isn't cached yet.
async fn ensure_media(args: &Args) -> anyhow::Result<PathBuf> {
    if let Some(media) = &args.media {
        anyhow::ensure!(media.is_file(), "media file not found: {}", media.display());
        return Ok(media.clone());
    }

    let clip = clip_cache_path(args);
    if clip.is_file() {
        return Ok(clip);
    }

    let ffmpeg = resolve_ffmpeg(args).await?;
    println!(
        "generating test clip {} with {}…",
        clip.display(),
        ffmpeg.bin.display()
    );
    let status = ffmpeg
        .command()
        .args([
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!(
            "testsrc2=size={}x{}:rate={}",
            args.width, args.height, args.fps
        ))
        .args(["-t", "20", "-c:v", "libx264", "-preset"])
        .arg(&args.preset)
        .args(["-pix_fmt", "yuv420p", "-g"])
        .arg(args.fps.to_string())
        .arg(&clip)
        .status()
        .await
        .with_context(|| format!("failed to run ffmpeg ({})", ffmpeg.bin.display()))?;
    anyhow::ensure!(status.success(), "ffmpeg exited with {status}");
    Ok(clip)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let media = ensure_media(&args).await?;
    let config_path = std::env::temp_dir().join(format!("dummy-rtsp-camera-{}.yaml", args.port));
    std::fs::write(&config_path, render_config(&args, &media))
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    let server_bin = resolve_server_bin(&args);
    println!("=== dummy-rtsp-camera (oddity-rtsp-server) ===");
    println!(
        "  rtsp url : rtsp://127.0.0.1:{}{}",
        args.port,
        normalize_path(&args.path)
    );
    println!("  media    : {}", media.display());
    println!("  config   : {}", config_path.display());
    // oddity-rtsp-server only accepts RTP over TCP (interleaved). Players
    // trying UDP first (ffplay/ffprobe defaults) log one "461 Unsupported
    // Transport" and then succeed on their TCP retry — harmless, but tell
    // the user how to silence it.
    println!("  note     : TCP interleaved only — use `ffplay -rtsp_transport tcp <url>`");

    let mut cmd = tokio::process::Command::new(&server_bin);
    cmd.arg(&config_path)
        .env(
            "LOG",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        )
        .kill_on_drop(true);
    if let Some(lib) = ffmpeg_lib_dir() {
        cmd.env(
            "LD_LIBRARY_PATH",
            prepend_ld_path(&lib, std::env::var_os("LD_LIBRARY_PATH")),
        );
    }
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!(
                "`{server_bin}` not found. Install it once, from the repo root \
                     (so .cargo/config.toml supplies FFMPEG_DIR=ffmpeg):\n  \
                     cargo install oddity-rtsp-server\n\
                     (or point --server-bin / $ODDITY_RTSP_SERVER_BIN at the binary)"
            )
        } else {
            anyhow::anyhow!("failed to spawn {server_bin}: {e}")
        }
    })?;

    tokio::select! {
        status = child.wait() => {
            let status = status.context("failed waiting for oddity-rtsp-server")?;
            anyhow::ensure!(status.success(), "oddity-rtsp-server exited with {status}");
        }
        _ = tokio::signal::ctrl_c() => {
            // The terminal delivers SIGINT to the whole process group, so the
            // server usually exits on its own; give it a moment, then make sure.
            let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
            let _ = child.kill().await;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "main_test.rs"]
mod main_test;
