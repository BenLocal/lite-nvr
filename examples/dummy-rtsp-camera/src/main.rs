//! A minimal RTSP camera emulator using `oddity-rtsp-protocol`.
//!
//! Starts an RTSP server on TCP, spawns an ffmpeg subprocess generating a
//! test-pattern H.264 Annex-B stream, and serves it to clients via RTP/UDP.
//!
//! ```bash
//! cargo run -p dummy-rtsp-camera
//! ffplay rtsp://127.0.0.1:8553/live/test1
//! ```

use std::io::{BufReader, Read};
use std::net::SocketAddr;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use oddity_rtsp_protocol::{
    AsServer, Codec, Method, Response, Status,
};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::Mutex;
use tokio::time::sleep;
use futures::{SinkExt, StreamExt};
use tokio_util::codec::Framed;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(name = "dummy-rtsp-camera")]
struct Args {
    /// RTSP listen address
    #[arg(long, default_value = "0.0.0.0")]
    listen: String,

    /// RTSP listen port
    #[arg(long, default_value_t = 8553)]
    port: u16,

    /// Stream path (e.g. /live/test1)
    #[arg(long, default_value = "/live/test1")]
    path: String,

    /// Video width
    #[arg(long, default_value_t = 1920)]
    width: u32,

    /// Video height
    #[arg(long, default_value_t = 1080)]
    height: u32,

    /// Frame rate
    #[arg(long, default_value_t = 25)]
    fps: u32,

    /// x264 preset
    #[arg(long, default_value = "ultrafast")]
    preset: String,

    /// Path to ffmpeg binary
    #[arg(long, default_value = "ffmpeg")]
    ffmpeg: String,
}

// ---------------------------------------------------------------------------
// NAL / Annex-B parsing
// ---------------------------------------------------------------------------

fn split_nalus(buf: &[u8]) -> Vec<Vec<u8>> {
    let mut nalus = Vec::new();
    let mut i = 0;
    while i + 3 <= buf.len() {
        let sc_len = if buf[i..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if buf[i..].starts_with(&[0, 0, 1]) {
            3
        } else {
            i += 1;
            continue;
        };
        let data_start = i + sc_len;
        // Find next start code
        let mut j = data_start;
        while j + 3 <= buf.len() {
            if buf[j..].starts_with(&[0, 0, 0, 1]) || buf[j..].starts_with(&[0, 0, 1]) {
                break;
            }
            j += 1;
        }
        if j > data_start {
            nalus.push(buf[data_start..j].to_vec());
        }
        i = j;
    }
    nalus
}

const NAL_SPS: u8 = 7;
const NAL_PPS: u8 = 8;

fn nal_type(nal: &[u8]) -> u8 {
    nal.first().map(|b| b & 0x1f).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// RTP packetizer
// ---------------------------------------------------------------------------

const RTP_PT: u8 = 96;
const MAX_RTP_PAYLOAD: usize = 1400;
const CLOCK_HZ: u64 = 90_000;

fn rtp_packet(seq: u16, ts: u32, ssrc: u32, marker: bool, payload: &[u8]) -> Vec<u8> {
    let mut pkt = vec![
        0x80,
        ((marker as u8) << 7) | RTP_PT,
        (seq >> 8) as u8,
        seq as u8,
        (ts >> 24) as u8,
        (ts >> 16) as u8,
        (ts >> 8) as u8,
        ts as u8,
        (ssrc >> 24) as u8,
        (ssrc >> 16) as u8,
        (ssrc >> 8) as u8,
        ssrc as u8,
    ];
    pkt.extend_from_slice(payload);
    pkt
}

fn send_nal(sock: &UdpSocket, dst: SocketAddr, seq: &mut u16, ts: u32, ssrc: u32, nal: &[u8]) {
    if nal.len() <= MAX_RTP_PAYLOAD {
        let _ = sock.try_send_to(&rtp_packet(*seq, ts, ssrc, true, nal), dst);
        *seq = seq.wrapping_add(1);
    } else {
        // FU-A fragmentation
        let header = nal[0];
        let nri = (header & 0x60) >> 5;
        let ntype = header & 0x1f;
        let fu_indicator = ((nri << 5) | 28) as u8;
        let data = &nal[1..];
        let chunks: Vec<&[u8]> = data.chunks(MAX_RTP_PAYLOAD - 2).collect();
        let total = chunks.len();
        for (i, chunk) in chunks.iter().enumerate() {
            let start = i == 0;
            let end = i + 1 == total;
            let fu_header = ((start as u8) << 7) | ((end as u8) << 6) | ntype;
            let mut payload = vec![fu_indicator, fu_header];
            payload.extend_from_slice(chunk);
            let _ = sock.try_send_to(&rtp_packet(*seq, ts, ssrc, end, &payload), dst);
            *seq = seq.wrapping_add(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Shared NAL buffer (ffmpeg stdout → all RTSP clients)
// ---------------------------------------------------------------------------

struct NalBuffer {
    nals: Vec<Vec<u8>>,
    sps: Option<Vec<u8>>,
    pps: Option<Vec<u8>>,
}

impl NalBuffer {
    fn push(&mut self, nal: Vec<u8>) {
        let nt = nal_type(&nal);
        if nt == NAL_SPS {
            self.sps = Some(nal.clone());
        } else if nt == NAL_PPS {
            self.pps = Some(nal.clone());
        }
        self.nals.push(nal);
    }

    fn take_nals(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.nals)
    }

    fn sps_pps_stap(&self) -> Option<Vec<u8>> {
        match (&self.sps, &self.pps) {
            (Some(s), Some(p)) => {
                let mut stap = vec![0x18u8]; // STAP-A NAL header
                stap.extend_from_slice(&(s.len() as u16).to_be_bytes());
                stap.extend_from_slice(s);
                stap.extend_from_slice(&(p.len() as u16).to_be_bytes());
                stap.extend_from_slice(p);
                Some(stap)
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// SDP
// ---------------------------------------------------------------------------

fn sdp() -> &'static str {
    "v=0\r\n\
     o=- 0 1 IN IP4 127.0.0.1\r\n\
     s=Test\r\n\
     c=IN IP4 0.0.0.0\r\n\
     t=0 0\r\n\
     m=video 0 RTP/AVP 96\r\n\
     a=rtpmap:96 H264/90000\r\n\
     a=fmtp:96 packetization-mode=1\r\n\
     a=control:trackID=0\r\n"
}

// ---------------------------------------------------------------------------
// RTSP session handler
// ---------------------------------------------------------------------------

struct Session {
    sock: Arc<UdpSocket>,
    dst: SocketAddr,
    ssrc: u32,
}

async fn handle_client(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    nal_buf: Arc<Mutex<NalBuffer>>,
    fps: u32,
) -> anyhow::Result<()> {

    let mut framed = Framed::new(stream, Codec::<AsServer>::new());
    let mut use_tcp = false;
    let mut ssrc: u32 = 0;

    tracing::info!("RTSP connect from {addr}");

    loop {
        let msg = match tokio::time::timeout(Duration::from_secs(30), framed.next()).await {
            Ok(Some(Ok(msg))) => msg,
            Ok(Some(Err(e))) => {
                tracing::warn!("RTSP parse error from {addr}: {e}");
                break;
            }
            _ => break,
        };
        let req = match msg {
            oddity_rtsp_protocol::MaybeInterleaved::Message(r) => r,
            _ => continue,
        };

        let resp = match req.method {
            Method::Options => Response::ok()
                .with_cseq_of(&req)
                .with_header("Public", "OPTIONS, DESCRIBE, SETUP, TEARDOWN, PLAY")
                .build(),

            Method::Describe => Response::ok()
                .with_cseq_of(&req)
                .with_sdp(sdp().to_string())
                .build(),

            Method::Setup => {
                let transports = req.transport().unwrap_or_default();
                tracing::info!(
                    "SETUP from {addr}: transports={transports:?}",
                );

                // Check if client wants TCP interleaved; we only support UDP.
                let wants_tcp = transports.iter().any(|t| {
                    t.lower_protocol().map_or(false, |l| matches!(l, oddity_rtsp_protocol::Lower::Tcp))
                });
                if wants_tcp {
                    let resp = Response::error(
                        oddity_rtsp_protocol::Status::UnsupportedTransport,
                    )
                    .with_cseq_of(&req)
                    .build();
                    framed.send(oddity_rtsp_protocol::MaybeInterleaved::Message(resp)).await?;
                    continue;
                }

                let client_port: u16 = transports
                    .iter()
                    .find_map(|t| t.client_port())
                    .and_then(|p| match p {
                        oddity_rtsp_protocol::Port::Range(from, _) => Some(*from),
                        _ => None,
                    })
                    .unwrap_or(0);

                let sock = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
                let server_port = sock.local_addr()?.port();
                let dst = SocketAddr::new(addr.ip(), client_port);
                let ssrc: u32 = rand::random();

                let sid = format!("{:08x}", rand::random::<u32>());
                Response::ok()
                    .with_cseq_of(&req)
                    .with_header(
                        "Transport",
                        format!(
                            "RTP/AVP/UDP;unicast;client_port={}-{};server_port={}-{}",
                            client_port, client_port + 1,
                            server_port, server_port + 1,
                        ),
                    )
                    .with_header("Session", &sid)
                    .build()
            }

            Method::Play => {
                if use_tcp {
                    // Transition into TCP interleaved streaming mode.
                    let resp = Response::ok()
                        .with_cseq_of(&req)
                        .with_header("Range", "npt=0.000-")
                        .build();
                    framed.send(oddity_rtsp_protocol::MaybeInterleaved::Message(resp)).await?;

                    // Stream RTP interleaved, checking for TEARDOWN between frames.
                    rtp_stream_tcp(&mut framed, nal_buf.clone(), ssrc, fps, addr).await?;
                    break;
                } else {
                    // UDP: response already handled in SETUP block
                    let resp = Response::ok()
                        .with_cseq_of(&req)
                        .with_header("Range", "npt=0.000-")
                        .build();
                    framed.send(oddity_rtsp_protocol::MaybeInterleaved::Message(resp)).await?;
                    // UDP streaming is handled by rtp_stream spawned in SETUP
                }
                continue;
            }

            Method::Teardown => {
                let resp = Response::ok().with_cseq_of(&req).build();
                framed.send(oddity_rtsp_protocol::MaybeInterleaved::Message(resp)).await?;
                break;
            }

            _ => Response::error(Status::NotImplemented)
                .with_cseq_of(&req)
                .build(),
        };

        framed.send(oddity_rtsp_protocol::MaybeInterleaved::Message(resp)).await?;
    }

    tracing::info!("RTSP disconnect from {addr}");
    Ok(())
}

// ---------------------------------------------------------------------------
// RTP streaming loop
// ---------------------------------------------------------------------------

async fn rtp_stream_tcp(
    framed: &mut Framed<tokio::net::TcpStream, Codec<AsServer>>,
    nal_buf: Arc<Mutex<NalBuffer>>,
    ssrc: u32,
    fps: u32,
    addr: SocketAddr,
) -> anyhow::Result<()> {
    use futures::StreamExt;

    let mut seq: u16 = 0;
    let mut ts: u32 = 0;
    let ts_step = (CLOCK_HZ / fps as u64) as u32;
    let frame_interval = Duration::from_secs_f64(1.0 / fps as f64);

    tracing::info!("RTP/TCP streaming to {addr}, ssrc={ssrc:#x}");

    // Wait for SPS/PPS
    for _ in 0..50 {
        if nal_buf.lock().await.sps.is_some() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    // Send SPS+PPS STAP-A
    if let Some(stap) = nal_buf.lock().await.sps_pps_stap() {
        let pkt = rtp_packet(seq, ts, ssrc, true, &stap);
        framed
            .send(oddity_rtsp_protocol::MaybeInterleaved::Interleaved {
                channel: 0,
                payload: pkt.into(),
            })
            .await?;
        seq = seq.wrapping_add(1);
    }

    let mut preamble: Vec<Vec<u8>> = Vec::new();

    loop {
        // Drain one access unit from the NAL buffer
        let au: Vec<Vec<u8>> = {
            let mut buf = nal_buf.lock().await;
            let nalus = buf.take_nals();
            if nalus.is_empty() {
                Vec::new()
            } else {
                let mut all = preamble.clone();
                preamble.clear();
                all.extend(nalus);
                if let Some(vcl_pos) = all.iter().position(|n| {
                    let t = nal_type(n);
                    t == 1 || t == 5
                }) {
                    let au: Vec<Vec<u8>> = all.drain(..=vcl_pos).collect();
                    preamble = all;
                    au
                } else {
                    preamble = all;
                    Vec::new()
                }
            }
        };

        if au.is_empty() {
            if let Some(stap) = nal_buf.lock().await.sps_pps_stap() {
                let pkt = rtp_packet(seq, ts, ssrc, true, &stap);
                framed
                    .send(oddity_rtsp_protocol::MaybeInterleaved::Interleaved {
                        channel: 0,
                        payload: pkt.into(),
                    })
                    .await?;
                seq = seq.wrapping_add(1);
            }
        } else {
            for nal in &au {
                let nal_data: Vec<u8> = nal.clone();
                if nal_data.len() <= MAX_RTP_PAYLOAD {
                    let pkt = rtp_packet(seq, ts, ssrc, true, &nal_data);
                    framed
                        .send(oddity_rtsp_protocol::MaybeInterleaved::Interleaved {
                            channel: 0,
                            payload: pkt.into(),
                        })
                        .await?;
                    seq = seq.wrapping_add(1);
                } else {
                    // FU-A fragmentation inline
                    let header = nal_data[0];
                    let nri = (header & 0x60) >> 5;
                    let ntype = header & 0x1f;
                    let fu_indicator = ((nri << 5) | 28) as u8;
                    let data = &nal_data[1..];
                    let chunks: Vec<&[u8]> = data.chunks(MAX_RTP_PAYLOAD - 2).collect();
                    let total = chunks.len();
                    for (i, chunk) in chunks.iter().enumerate() {
                        let end = i + 1 == total;
                        let fu_header = ((i == 0) as u8) << 7 | (end as u8) << 6 | ntype;
                        let mut payload = vec![fu_indicator, fu_header];
                        payload.extend_from_slice(chunk);
                        let pkt = rtp_packet(seq, ts, ssrc, end, &payload);
                        framed
                            .send(oddity_rtsp_protocol::MaybeInterleaved::Interleaved {
                                channel: 0,
                                payload: pkt.into(),
                            })
                            .await?;
                        seq = seq.wrapping_add(1);
                    }
                }
            }
            ts = ts.wrapping_add(ts_step);
        }

        // Check for TEARDOWN between frames (non-blocking)
        match tokio::time::timeout(Duration::from_millis(1), framed.next()).await {
            Ok(Some(Ok(msg))) => {
                if let oddity_rtsp_protocol::MaybeInterleaved::Message(req) = msg {
                    if req.method == Method::Teardown {
                        let resp = Response::ok().with_cseq_of(&req).build();
                        framed
                            .send(oddity_rtsp_protocol::MaybeInterleaved::Message(resp))
                            .await?;
                        break;
                    }
                }
            }
            _ => {}
        }

        sleep(frame_interval).await;
    }

    tracing::info!("RTP/TCP streaming to {addr} ended");
    Ok(())
}

async fn rtp_stream(
    sock: Arc<UdpSocket>,
    dst: SocketAddr,
    ssrc: u32,
    nal_buf: Arc<Mutex<NalBuffer>>,
    fps: u32,
) {
    let mut seq: u16 = 0;
    let mut ts: u32 = 0;
    let ts_step = (CLOCK_HZ / fps as u64) as u32;
    let frame_interval = Duration::from_secs_f64(1.0 / fps as f64);

    tracing::info!("RTP streaming to {dst}, ssrc={ssrc:#x}");

    // Wait for SPS/PPS
    for _ in 0..50 {
        if nal_buf.lock().await.sps.is_some() {
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }

    // Send SPS+PPS STAP-A as first packet
    if let Some(stap) = nal_buf.lock().await.sps_pps_stap() {
        seq = send_nal_r(&sock, dst, seq, ts, ssrc, &stap);
    }

    // Non-VCL NALs accumulated before the current frame\'s VCL NAL.
    let mut preamble: Vec<Vec<u8>> = Vec::new();

    loop {
        // Extract one access unit: all non-VCL NALs + the next VCL NAL.
        let au: Vec<Vec<u8>> = {
            let mut buf = nal_buf.lock().await;
            let nalus = buf.take_nals();
            if nalus.is_empty() {
                Vec::new()
            } else {
                // Prepend any non-VCL NALs retained from last iteration.
                let mut all = preamble.clone();
                preamble.clear();
                all.extend(nalus);
                // Split at the first VCL NAL (type 1=P, 5=IDR).
                if let Some(vcl_pos) = all.iter().position(|n| {
                    let t = nal_type(n);
                    t == 1 || t == 5
                }) {
                    // Everything up to and including the VCL NAL is one frame.
                    let au: Vec<Vec<u8>> = all.drain(..=vcl_pos).collect();
                    // The rest (non-VCL before next frame) is preamble.
                    preamble = all;
                    au
                } else {
                    // No VCL NAL yet; keep everything as preamble for next time.
                    preamble = all;
                    Vec::new()
                }
            }
        };

        if au.is_empty() {
            // Filler: re-send SPS/PPS
            if let Some(stap) = nal_buf.lock().await.sps_pps_stap() {
                seq = send_nal_r(&sock, dst, seq, ts, ssrc, &stap);
            }
        } else {
            for nal in &au {
                seq = send_nal_r(&sock, dst, seq, ts, ssrc, nal);
            }
            ts = ts.wrapping_add(ts_step);
        }

        sleep(frame_interval).await;
    }
}

fn send_nal_r(sock: &UdpSocket, dst: SocketAddr, seq: u16, ts: u32, ssrc: u32, nal: &[u8]) -> u16 {
    let mut s = seq;
    send_nal(sock, dst, &mut s, ts, ssrc, nal);
    s
}

// ---------------------------------------------------------------------------
// FFmpeg reader thread
// ---------------------------------------------------------------------------

fn start_ffmpeg(args: &Args, nal_buf: Arc<Mutex<NalBuffer>>) -> anyhow::Result<()> {
    let mut child = Command::new(&args.ffmpeg)
        .args([
            "-re",
            "-f", "lavfi",
            "-i", &format!("testsrc=size={}x{}:rate={}", args.width, args.height, args.fps),
            "-c:v", "libx264",
            "-preset", &args.preset,
            "-tune", "zerolatency",
            "-pix_fmt", "yuv420p",
            "-an",
            "-f", "h264",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .context("spawn ffmpeg")?;

    let stdout = child.stdout.take().unwrap();

    std::thread::spawn(move || {
        let mut reader = BufReader::with_capacity(256 * 1024, stdout);
        let mut buf = Vec::with_capacity(256 * 1024);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();

        loop {
            let mut chunk = vec![0u8; 65536];
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    chunk.truncate(n);
                    buf.extend_from_slice(&chunk);

                    // Find all Annex-B start code positions (00 00 00 01 or 00 00 01).
                    let mut starts: Vec<usize> = Vec::new();
                    let mut i = 0;
                    while i + 3 <= buf.len() {
                        if buf[i..].starts_with(&[0, 0, 0, 1]) {
                            starts.push(i);
                            i += 4;
                        } else if buf[i..].starts_with(&[0, 0, 1]) {
                            starts.push(i);
                            i += 3;
                        } else {
                            i += 1;
                        }
                    }

                    // We need at least 2 start codes to have a complete NAL.
                    // Everything before the last start code is complete; keep the
                    // rest (partial NAL) for the next read.
                    if starts.len() >= 2 {
                        let last_start = *starts.last().unwrap();
                        let complete = buf[..last_start].to_vec();
                        buf = buf[last_start..].to_vec();

                        let nalus = split_nalus(&complete);
                        if !nalus.is_empty() {
                            runtime.block_on(async {
                                let mut nb = nal_buf.lock().await;
                                for nal in &nalus {
                                    nb.push(nal.clone());
                                }
                            });
                        }
                    }
                }
                Err(_) => break,
            }
        }
        tracing::info!("ffmpeg stdout ended");
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,dummy_rtsp_camera=info".into()),
        )
        .init();

    let args = Args::parse();
    let nal_buf = Arc::new(Mutex::new(NalBuffer {
        nals: Vec::new(),
        sps: None,
        pps: None,
    }));

    start_ffmpeg(&args, nal_buf.clone())?;

    let bind = format!("{}:{}", args.listen, args.port);
    let listener = TcpListener::bind(&bind)
        .await
        .with_context(|| format!("bind {bind}"))?;
    tracing::info!(
        "RTSP server listening on rtsp://127.0.0.1:{}{}",
        args.port,
        args.path
    );

    loop {
        let (stream, addr) = listener.accept().await?;
        let buf = nal_buf.clone();
        let fps = args.fps;
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, addr, buf, fps).await {
                tracing::warn!("RTSP session error ({addr}): {e:#}");
            }
        });
    }
}
