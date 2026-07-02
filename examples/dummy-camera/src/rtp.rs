//! RTP packetization (PT 96, PS payload) and the media wire. GB28181 carries the
//! Program Stream over RTP; each picture's PS bytes are fragmented into RTP
//! packets sharing one 90 kHz timestamp, marker set on the last fragment.
//!
//! Transport mirrors the platform's role in the SDP offer:
//! - **UDP** — we send datagrams to the platform's receive address.
//! - **TCP passive** — the platform listens; we connect out and length-prefix
//!   each packet (RFC 4571).
//! - **TCP active** — the platform connects to us; we listen, accept, then send.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use gb28181::Transport;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream, UdpSocket};

use crate::h264::AccessUnit;
use crate::ps;

const RTP_PT: u8 = 96; // GB28181 PS payload type
const MAX_PAYLOAD: usize = 1400; // keep RTP packets under a typical MTU
const CLOCK_HZ: u64 = 90_000; // RTP/PS video clock

/// A bound local endpoint plus the pending action needed to reach the platform.
pub enum PreparedWire {
    Udp { sock: UdpSocket, dst: SocketAddr },
    TcpConnect { dst: SocketAddr },
    TcpListen { listener: TcpListener },
}

/// Bind the local socket the transport needs and return it together with the
/// `ip:port` to advertise as our media source in the SDP answer.
pub async fn setup_wire(
    transport: Transport,
    dst: SocketAddr,
    media_ip: IpAddr,
    media_port: u16,
) -> Result<(PreparedWire, SocketAddr)> {
    match transport {
        Transport::Udp => {
            let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, media_port))
                .await
                .context("bind udp media socket")?;
            let port = sock.local_addr()?.port();
            Ok((
                PreparedWire::Udp { sock, dst },
                SocketAddr::new(media_ip, port),
            ))
        }
        Transport::TcpPassive => {
            // Platform listens; we connect out. Advertised addr is informational.
            let port = if media_port == 0 { 5062 } else { media_port };
            Ok((
                PreparedWire::TcpConnect { dst },
                SocketAddr::new(media_ip, port),
            ))
        }
        Transport::TcpActive => {
            // Platform connects to us; advertise the port we listen on.
            let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, media_port))
                .await
                .context("bind tcp media listener")?;
            let port = listener.local_addr()?.port();
            Ok((
                PreparedWire::TcpListen { listener },
                SocketAddr::new(media_ip, port),
            ))
        }
    }
}

enum Sink {
    Udp { sock: UdpSocket, dst: SocketAddr },
    Tcp(TcpStream),
}

impl Sink {
    /// Complete the connection: connect out (passive platform) or accept the
    /// platform's connection (active platform); UDP is already ready.
    async fn finalize(wire: PreparedWire) -> Result<Sink> {
        Ok(match wire {
            PreparedWire::Udp { sock, dst } => Sink::Udp { sock, dst },
            PreparedWire::TcpConnect { dst } => Sink::Tcp(
                TcpStream::connect(dst)
                    .await
                    .context("tcp connect to platform")?,
            ),
            PreparedWire::TcpListen { listener } => {
                let (stream, _peer) =
                    tokio::time::timeout(Duration::from_secs(10), listener.accept())
                        .await
                        .context("timed out waiting for platform tcp connect")?
                        .context("accept platform tcp connect")?;
                Sink::Tcp(stream)
            }
        })
    }

    async fn send(&mut self, pkt: &[u8]) -> Result<()> {
        match self {
            Sink::Udp { sock, dst } => {
                sock.send_to(pkt, *dst).await?;
            }
            Sink::Tcp(stream) => {
                stream.write_all(&(pkt.len() as u16).to_be_bytes()).await?;
                stream.write_all(pkt).await?;
            }
        }
        Ok(())
    }
}

/// Muxes access units to PS/RTP over a finalized wire, tracking sequence and the
/// 90 kHz timestamp across calls.
pub struct RtpStreamer {
    sink: Sink,
    ssrc: u32,
    seq: u16,
    ts: u32,
    step: u32,
}

impl RtpStreamer {
    pub async fn new(wire: PreparedWire, ssrc: u32, fps: u32) -> Result<Self> {
        Ok(Self {
            sink: Sink::finalize(wire).await?,
            ssrc,
            seq: 0,
            ts: 0,
            step: (CLOCK_HZ / fps.max(1) as u64) as u32,
        })
    }

    /// Mux one access unit to PS and send it as RTP (marker on the last frag).
    pub async fn send_au(&mut self, au: &AccessUnit) -> Result<()> {
        let ps = ps::mux_access_unit(au, self.ts as u64);
        send_rtp(&mut self.sink, &ps, self.ssrc, self.ts, &mut self.seq).await?;
        self.ts = self.ts.wrapping_add(self.step);
        Ok(())
    }
}

/// Loop the bundled access units forever, muxed to PS/RTP, paced at `fps`, until
/// the task is aborted (on session close).
pub async fn stream_access_units(
    wire: PreparedWire,
    aus: Arc<Vec<AccessUnit>>,
    ssrc: u32,
    fps: u32,
) -> Result<()> {
    if aus.is_empty() {
        anyhow::bail!("no access units to stream");
    }
    let mut streamer = RtpStreamer::new(wire, ssrc, fps).await?;
    let mut ticker = tokio::time::interval(Duration::from_micros(1_000_000 / fps.max(1) as u64));
    loop {
        for au in aus.iter() {
            ticker.tick().await;
            streamer.send_au(au).await?;
        }
    }
}

/// How ffmpeg should read the source file.
pub enum PlayMode {
    /// Loop the whole file forever (live `--source-file`).
    LiveLoop,
    /// Play once, seeked `seek` seconds in, for `dur` seconds (`None` = to end).
    /// Used for Playback / Download of a recorded window.
    Segment { seek: u64, dur: Option<u64> },
}

/// Stream a local video file via ffmpeg: transcode to Annex-B H.264 (real-time
/// paced with `-re`), parse access units incrementally, and push them as PS/RTP.
/// Returns when ffmpeg ends (segment/file done) or on error.
pub async fn stream_ffmpeg_file(
    wire: PreparedWire,
    file: &str,
    ssrc: u32,
    fps: u32,
    mode: PlayMode,
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    use tokio::process::Command;

    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-hide_banner", "-loglevel", "error", "-re"]);
    if matches!(mode, PlayMode::LiveLoop) {
        cmd.args(["-stream_loop", "-1"]);
    }
    if let PlayMode::Segment { seek, .. } = &mode
        && *seek > 0
    {
        cmd.args(["-ss", &seek.to_string()]);
    }
    cmd.args(["-i", file]);
    if let PlayMode::Segment { dur: Some(t), .. } = &mode {
        cmd.args(["-t", &t.to_string()]);
    }
    cmd.args([
        "-an",
        "-c:v",
        "libx264",
        "-profile:v",
        "baseline",
        "-pix_fmt",
        "yuv420p",
    ]);
    cmd.args([
        "-x264-params",
        &format!("keyint={fps}:scenecut=0:repeat-headers=1"),
    ]);
    cmd.args(["-f", "h264", "pipe:1"]);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());

    let mut child = cmd.spawn().context("spawn ffmpeg (is it on PATH?)")?;
    let mut stdout = child.stdout.take().expect("piped stdout");

    let mut streamer = RtpStreamer::new(wire, ssrc, fps).await?;
    let mut parser = crate::h264::AnnexBParser::new();
    let mut buf = vec![0u8; 65536];
    loop {
        let n = stdout.read(&mut buf).await.context("read ffmpeg stdout")?;
        if n == 0 {
            break; // ffmpeg finished (playback range done, or file end)
        }
        for au in parser.push(&buf[..n]) {
            streamer.send_au(&au).await?;
        }
    }
    let _ = child.kill().await;
    Ok(())
}

/// Fragment a PS buffer into RTP packets (marker bit on the last) and send them.
async fn send_rtp(sink: &mut Sink, ps: &[u8], ssrc: u32, ts: u32, seq: &mut u16) -> Result<()> {
    let mut off = 0;
    while off < ps.len() {
        let end = (off + MAX_PAYLOAD).min(ps.len());
        let marker = end == ps.len();
        let mut pkt = Vec::with_capacity(12 + (end - off));
        pkt.push(0x80); // V=2, no padding/extension/CSRC
        pkt.push(RTP_PT | if marker { 0x80 } else { 0 });
        pkt.extend_from_slice(&seq.to_be_bytes());
        pkt.extend_from_slice(&ts.to_be_bytes());
        pkt.extend_from_slice(&ssrc.to_be_bytes());
        pkt.extend_from_slice(&ps[off..end]);
        sink.send(&pkt).await?;
        *seq = seq.wrapping_add(1);
        off = end;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn udp_fragments_carry_rtp_header_and_marker() {
        // Receiver socket stands in for the platform's RtpServer.
        let rx = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let dst = rx.local_addr().unwrap();
        let mut sink = Sink::Udp {
            sock: UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap(),
            dst,
        };
        // Two fragments: MAX_PAYLOAD + a little.
        let ps = vec![0x5A; MAX_PAYLOAD + 10];
        let mut seq = 7;
        send_rtp(&mut sink, &ps, 0x0102_0304, 90_000, &mut seq)
            .await
            .unwrap();

        let mut buf = [0u8; 2048];
        let n1 = rx.recv(&mut buf).await.unwrap();
        assert_eq!(buf[0], 0x80);
        assert_eq!(buf[1] & 0x7F, RTP_PT);
        assert_eq!(buf[1] & 0x80, 0); // first fragment: marker clear
        assert_eq!(u16::from_be_bytes([buf[2], buf[3]]), 7);
        assert_eq!(u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]), 90_000);
        assert_eq!(
            u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]),
            0x0102_0304
        );
        assert_eq!(n1, 12 + MAX_PAYLOAD);

        let n2 = rx.recv(&mut buf).await.unwrap();
        assert_eq!(buf[1] & 0x80, 0x80); // last fragment: marker set
        assert_eq!(u16::from_be_bytes([buf[2], buf[3]]), 8); // seq advanced
        assert_eq!(n2, 12 + 10);
        assert_eq!(seq, 9);
    }
}
