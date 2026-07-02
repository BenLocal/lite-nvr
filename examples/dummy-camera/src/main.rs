//! A GB28181 device (下级设备) emulator — a "dummy camera".
//!
//! It behaves like a real Hikvision IPC toward an NVR/platform: it REGISTERs
//! (with digest auth), keeps alive, answers Catalog queries, and on INVITE
//! pushes a looping H.264 clip as PS-over-RTP to the platform's receive address.
//! DeviceControl (PTZ) commands are logged (a real camera would move).
//!
//! The signaling is all handled by the `gb28181` crate's `GbClient`; this binary
//! adds the media plane (see `h264`/`ps`/`rtp`) and the glue.
//!
//! ```text
//! cargo run -p dummy-camera -- \
//!   --server-addr 127.0.0.1:5060 \
//!   --server-id 34020000002000000001 \
//!   --device-id 34020000001320000001 \
//!   --password 12345678
//! ```

mod h264;
mod ps;
mod rtp;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use gb28181::client::InviteNegotiation;
use gb28181::manscdp::{CatalogItem, RecordItem};
use gb28181::{GbClient, GbClientConfig, GbEvent};
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// The bundled sample clip (raw Annex-B H.264), looped as the camera's video.
const SAMPLE_H264: &[u8] = include_bytes!("../assets/sample.h264");

#[derive(Parser, Debug)]
#[command(
    name = "dummy-camera",
    about = "GB28181 device emulator: registers to an NVR and streams PS/RTP like a real IPC"
)]
struct Args {
    /// Platform (NVR) SIP UDP address, e.g. 127.0.0.1:5060.
    #[arg(long)]
    server_addr: SocketAddr,

    /// Platform's 20-digit GB code (Request-URI user for REGISTER/MESSAGE).
    #[arg(long)]
    server_id: String,

    /// Our device's 20-digit GB code.
    #[arg(long)]
    device_id: String,

    /// SIP domain / digest realm. Defaults to the first 10 digits of --server-id.
    #[arg(long)]
    domain: Option<String>,

    /// Digest password (omit for an open/no-auth platform).
    #[arg(long)]
    password: Option<String>,

    /// Advertised channel GB code (the Catalog entry the NVR pulls). Defaults to
    /// --device-id (a single-channel device that is its own channel).
    #[arg(long)]
    channel_id: Option<String>,

    /// Human-readable channel name in the Catalog.
    #[arg(long, default_value = "dummy-camera")]
    channel_name: String,

    /// IP to advertise as our media source in the SDP answer.
    #[arg(long, default_value = "127.0.0.1")]
    media_ip: IpAddr,

    /// Local media port (0 = ephemeral, recommended so concurrent pulls don't clash).
    #[arg(long, default_value_t = 0)]
    media_port: u16,

    /// Local SIP listen address.
    #[arg(long, default_value = "0.0.0.0:5061")]
    listen: SocketAddr,

    /// Playback frame rate (paces RTP and advances the 90 kHz timestamp).
    #[arg(long, default_value_t = 25)]
    fps: u32,

    /// Registration expiry (seconds).
    #[arg(long, default_value_t = 3600)]
    expires: u32,

    /// Keepalive interval (seconds).
    #[arg(long, default_value_t = 60)]
    keepalive: u64,

    /// Local video file to stream live on a Play INVITE (read by ffmpeg, looped)
    /// instead of the bundled clip. Any format ffmpeg reads; transcoded to H.264.
    #[arg(long)]
    source_file: Option<String>,

    /// Video file to advertise as a recording and serve on a Playback INVITE.
    /// Defaults to --source-file when unset (no recording advertised if neither).
    #[arg(long)]
    record_file: Option<String>,

    /// Advertised recording start time (ISO 8601) for RecordInfo + Playback seek.
    #[arg(long, default_value = "2024-01-01T00:00:00")]
    record_start: String,

    /// Advertised recording end time (ISO 8601).
    #[arg(long, default_value = "2024-01-01T01:00:00")]
    record_end: String,

    /// DeviceInfo manufacturer reported to the platform.
    #[arg(long, default_value = "lite-nvr")]
    manufacturer: String,

    /// DeviceInfo model reported to the platform.
    #[arg(long, default_value = "dummy-camera")]
    model: String,

    /// DeviceInfo firmware version reported to the platform.
    #[arg(long, default_value = "0.1")]
    firmware: String,

    /// Diagnostic: mux the bundled clip to a Program Stream file and exit (no
    /// SIP). Inspect with `ffprobe <file>` to confirm the PS/H.264 framing.
    #[arg(long)]
    dump_ps: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,dummy_camera=info".into()),
        )
        .init();

    let args = Args::parse();
    run(args).await
}

async fn run(args: Args) -> Result<()> {
    let domain = args.domain.clone().unwrap_or_else(|| {
        args.server_id
            .get(..10)
            .unwrap_or(&args.server_id)
            .to_string()
    });
    let channel_id = args
        .channel_id
        .clone()
        .unwrap_or_else(|| args.device_id.clone());

    // Parse the bundled clip into access units once; the stream tasks loop it.
    let aus = Arc::new(h264::parse_access_units(SAMPLE_H264));
    info!(
        "loaded sample clip: {} access units ({} keyframes)",
        aus.len(),
        aus.iter().filter(|a| a.keyframe).count()
    );
    if aus.is_empty() {
        anyhow::bail!("bundled sample.h264 contained no access units");
    }

    if let Some(path) = &args.dump_ps {
        let step = (90_000 / args.fps.max(1)) as u64;
        let mut out = Vec::new();
        let mut pts = 0u64;
        for au in aus.iter() {
            out.extend_from_slice(&ps::mux_access_unit(au, pts));
            pts += step;
        }
        std::fs::write(path, &out).with_context(|| format!("write {}", path.display()))?;
        info!(
            "wrote {} bytes of Program Stream to {}",
            out.len(),
            path.display()
        );
        return Ok(());
    }

    let mut cfg = GbClientConfig::new(
        args.device_id.clone(),
        domain.clone(),
        args.server_id.clone(),
        args.server_addr,
    );
    cfg.password = args.password.clone();
    cfg.listen = args.listen;
    cfg.expires = args.expires;
    cfg.keepalive_interval = Duration::from_secs(args.keepalive.max(1));
    cfg.user_agent = "dummy-camera/0.1 (lite-nvr example)".into();
    cfg.channels = vec![CatalogItem {
        device_id: channel_id.clone(),
        name: args.channel_name.clone(),
        status: "ON".into(),
    }];
    cfg.device_name = args.channel_name.clone();
    cfg.manufacturer = args.manufacturer.clone();
    cfg.model = args.model.clone();
    cfg.firmware = args.firmware.clone();
    // A recording we can serve for Playback: explicit --record-file, else the
    // live --source-file. Advertised as one RecordInfo entry if present.
    let record_file = args
        .record_file
        .clone()
        .or_else(|| args.source_file.clone());
    if let Some(file) = &record_file {
        cfg.records = vec![RecordItem {
            device_id: channel_id.clone(),
            name: args.channel_name.clone(),
            file_path: file.clone(),
            start_time: args.record_start.clone(),
            end_time: args.record_end.clone(),
        }];
    }

    let (client, mut events) = GbClient::bind(cfg).await.context("bind SIP client")?;
    info!(
        "SIP bound on {} — device {} channel {} -> platform {} @ {}",
        client.local_addr(),
        args.device_id,
        channel_id,
        args.server_id,
        args.server_addr
    );

    // Register, but let Ctrl-C during the (retrying) registration exit cleanly.
    tokio::select! {
        r = register_with_retry(&client, &args.device_id) => r?,
        _ = tokio::signal::ctrl_c() => {
            info!("interrupted before registration — exiting");
            client.shutdown();
            return Ok(());
        }
    }

    let media = MediaCfg {
        media_ip: args.media_ip,
        media_port: args.media_port,
        fps: args.fps,
        source_file: args.source_file.clone(),
        record_file,
        record_start_unix: iso_to_unix(&args.record_start).unwrap_or(0),
        aus,
    };

    // dialog_id -> (streaming task, media handle). Keeps the pull alive until BYE.
    let mut streams: HashMap<String, (JoinHandle<()>, gb28181::ClientMediaHandle)> = HashMap::new();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("interrupted — unregistering (设备注销)");
                match client.unregister().await {
                    Ok(()) => info!("unregistered — bye"),
                    Err(e) => warn!(error = %e, "unregister failed"),
                }
                break;
            }
            ev = events.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    GbEvent::InviteReceived(neg) => match handle_invite(neg, &media).await {
                        Ok(Some((dialog_id, task, handle))) => {
                            streams.insert(dialog_id, (task, handle));
                        }
                        Ok(None) => {} // rejected (e.g. Playback with no recording)
                        Err(e) => {
                            warn!(error = %format!("{e:#}"), "failed to start media for INVITE")
                        }
                    },
                    GbEvent::SessionClosed { dialog_id } => {
                        if let Some((task, _handle)) = streams.remove(&dialog_id) {
                            task.abort();
                            info!("session {dialog_id} closed — stopped streaming");
                        }
                    }
                    GbEvent::DeviceControlReceived { device_id, ptz_cmd } => {
                        // A real camera would drive its motors; we just log it.
                        info!("PTZ command for {device_id}: {ptz_cmd}");
                    }
                    other => info!("event: {other:?}"),
                }
            }
        }
    }

    client.shutdown();
    Ok(())
}

/// Media-plane configuration threaded into each INVITE handler.
struct MediaCfg {
    media_ip: IpAddr,
    media_port: u16,
    fps: u32,
    /// Live Play source (ffmpeg, looped); `None` uses the bundled clip.
    source_file: Option<String>,
    /// Playback source (ffmpeg, seeked); `None` rejects Playback INVITEs.
    record_file: Option<String>,
    /// Unix seconds of `record_start`, to offset the Playback seek.
    record_start_unix: u64,
    /// The bundled clip's access units (live fallback).
    aus: Arc<Vec<h264::AccessUnit>>,
}

/// Parse an ISO-8601 `YYYY-MM-DDTHH:MM:SS` timestamp to Unix seconds (UTC).
fn iso_to_unix(s: &str) -> Option<u64> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc().timestamp().max(0) as u64)
}

/// Register, retrying every 5s until the platform accepts (like a real camera
/// coming up before its NVR). Cancelled by Ctrl-C via the outer `select!`.
async fn register_with_retry(client: &GbClient, device_id: &str) -> Result<()> {
    loop {
        match client.register().await {
            Ok(()) => {
                info!("registered as {device_id} — keepalive started");
                return Ok(());
            }
            Err(e) => {
                warn!(error = %e, "register failed; retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

/// What to stream for an INVITE, decided before answering so a Playback with no
/// recording can be rejected cleanly.
enum StreamAction {
    /// Loop the bundled clip (live Play, no --source-file).
    Bundled,
    /// ffmpeg-loop a local file forever (live Play with --source-file).
    LiveFile(String),
    /// ffmpeg a local file once, seeked to a window (Playback / Download).
    Playback {
        file: String,
        seek: u64,
        dur: Option<u64>,
    },
}

/// Set up the media wire, answer the INVITE naming our source address, and spawn
/// the paced streaming task. Returns `None` when the INVITE was rejected (e.g. a
/// Playback with no configured recording); otherwise the dialog id + task so the
/// caller can stop it on session close.
async fn handle_invite(
    neg: InviteNegotiation,
    mc: &MediaCfg,
) -> Result<Option<(String, JoinHandle<()>, gb28181::ClientMediaHandle)>> {
    let transport = neg.remote.transport;
    let dst = neg.remote.media_addr;
    let ssrc = neg
        .remote
        .ssrc
        .as_deref()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let session = neg.remote.session.clone();
    let (start, stop) = (neg.remote.start, neg.remote.stop);
    let dialog_id = neg.dialog_id();
    let is_playback =
        session.eq_ignore_ascii_case("Playback") || session.eq_ignore_ascii_case("Download");

    let action = if is_playback {
        match &mc.record_file {
            Some(file) => StreamAction::Playback {
                file: file.clone(),
                // Offset into the file for the requested window start.
                seek: start.saturating_sub(mc.record_start_unix),
                dur: (stop > start).then(|| stop - start),
            },
            None => {
                info!("Playback INVITE {dialog_id} but no recording configured — rejecting");
                neg.reject().ok();
                return Ok(None);
            }
        }
    } else if let Some(file) = &mc.source_file {
        StreamAction::LiveFile(file.clone())
    } else {
        StreamAction::Bundled
    };

    let (wire, advertised) = rtp::setup_wire(transport, dst, mc.media_ip, mc.media_port).await?;
    info!(
        "INVITE {dialog_id}: {session} {transport:?} -> {dst} (ssrc {ssrc}), answering as {advertised}"
    );
    let handle = neg.answer(advertised).context("answer INVITE")?;

    let (fps, aus) = (mc.fps, mc.aus.clone());
    let task = tokio::spawn(async move {
        let r = match action {
            StreamAction::Bundled => rtp::stream_access_units(wire, aus, ssrc, fps).await,
            StreamAction::LiveFile(file) => {
                rtp::stream_ffmpeg_file(wire, &file, ssrc, fps, rtp::PlayMode::LiveLoop).await
            }
            StreamAction::Playback { file, seek, dur } => {
                rtp::stream_ffmpeg_file(
                    wire,
                    &file,
                    ssrc,
                    fps,
                    rtp::PlayMode::Segment { seek, dur },
                )
                .await
            }
        };
        if let Err(e) = r {
            warn!(error = %format!("{e:#}"), "media stream ended with error");
        }
    });
    Ok(Some((dialog_id, task, handle)))
}
