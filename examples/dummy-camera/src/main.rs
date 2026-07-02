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
use gb28181::manscdp::CatalogItem;
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
    tokio::select! {
        r = run(args) => r,
        _ = tokio::signal::ctrl_c() => {
            info!("interrupted — shutting down");
            Ok(())
        }
    }
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

    let (client, mut events) = GbClient::bind(cfg).await.context("bind SIP client")?;
    info!(
        "SIP bound on {} — device {} channel {} -> platform {} @ {}",
        client.local_addr(),
        args.device_id,
        channel_id,
        args.server_id,
        args.server_addr
    );

    register_with_retry(&client, &args.device_id).await?;

    // dialog_id -> (streaming task, media handle). Keeps the pull alive until BYE.
    let mut streams: HashMap<String, (JoinHandle<()>, gb28181::ClientMediaHandle)> = HashMap::new();

    while let Some(ev) = events.recv().await {
        match ev {
            GbEvent::InviteReceived(neg) => {
                match handle_invite(neg, aus.clone(), args.media_ip, args.media_port, args.fps)
                    .await
                {
                    Ok((dialog_id, task, handle)) => {
                        streams.insert(dialog_id, (task, handle));
                    }
                    Err(e) => warn!(error = %format!("{e:#}"), "failed to start media for INVITE"),
                }
            }
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

    client.shutdown();
    Ok(())
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

/// Set up the media wire, answer the INVITE naming our source address, and spawn
/// the paced streaming task. Returns the dialog id so the caller can stop it on
/// session close.
async fn handle_invite(
    neg: InviteNegotiation,
    aus: Arc<Vec<h264::AccessUnit>>,
    media_ip: IpAddr,
    media_port: u16,
    fps: u32,
) -> Result<(String, JoinHandle<()>, gb28181::ClientMediaHandle)> {
    let transport = neg.remote.transport;
    let dst = neg.remote.media_addr;
    let ssrc = neg
        .remote
        .ssrc
        .as_deref()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let dialog_id = neg.dialog_id();

    let (wire, advertised) = rtp::setup_wire(transport, dst, media_ip, media_port).await?;
    info!(
        "INVITE {dialog_id}: {transport:?}, pushing PS/RTP to {dst} (ssrc {ssrc}), \
         answering as {advertised}"
    );
    let handle = neg.answer(advertised).context("answer INVITE")?;

    let task = tokio::spawn(async move {
        if let Err(e) = rtp::stream_access_units(wire, aus, ssrc, fps).await {
            warn!(error = %format!("{e:#}"), "media stream ended with error");
        }
    });
    Ok((dialog_id, task, handle))
}
