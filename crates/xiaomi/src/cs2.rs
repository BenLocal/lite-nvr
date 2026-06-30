//! Port of go2rtc `pkg/xiaomi/miss/cs2/conn.go` — TUTK CS2 P2P transport.
//!
//! UDP (with seq-reordering + ACK reliability) or TCP fallback, established via
//! the CS2 LAN-search/punch handshake. A worker thread reads frames and routes
//! them to per-channel queues; `read_command`/`write_command` use channel 0,
//! `read_packet`/`write_packet` use channels 2/3.
//!
//! BLIND PORT: faithful to the Go source but not yet validated against a real
//! camera (the P2P handshake needs the device + TUTK relay responding).

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};

const MAGIC: u8 = 0xF1;
const MAGIC_DRW: u8 = 0xD1;
const MAGIC_TCP: u8 = 0x68;
const MSG_LAN_SEARCH: u8 = 0x30;
const MSG_PUNCH_PKT: u8 = 0x41;
const MSG_P2P_RDY_UDP: u8 = 0x42;
const MSG_P2P_RDY_TCP: u8 = 0x43;
const MSG_DRW: u8 = 0xD0;
const MSG_DRW_ACK: u8 = 0xD1;
const MSG_PING: u8 = 0xE0;
const MSG_PONG: u8 = 0xE1;

const HDR_SIZE: usize = 32;

/// Shared transport (UDP or TCP). `Read`/`Write` for both `UdpSocket` and
/// `&TcpStream` take `&self`, so an `Arc` can be shared between the reader
/// (worker thread) and the writers.
#[derive(Clone)]
enum Transport {
    Udp {
        sock: Arc<UdpSocket>,
        addr: SocketAddr,
    },
    Tcp {
        stream: Arc<TcpStream>,
    },
}

impl Transport {
    fn is_tcp(&self) -> bool {
        matches!(self, Transport::Tcp { .. })
    }

    fn write(&self, buf: &[u8]) -> io::Result<()> {
        match self {
            Transport::Udp { sock, addr } => {
                sock.send_to(buf, addr)?;
                Ok(())
            }
            Transport::Tcp { stream } => {
                // TCP framing: [u16 len][magicTCP][5 pad][payload]
                let mut framed = vec![0u8; 8 + buf.len()];
                framed[..2].copy_from_slice(&(buf.len() as u16).to_be_bytes());
                framed[2] = MAGIC_TCP;
                framed[8..].copy_from_slice(buf);
                (&**stream).write_all(&framed)
            }
        }
    }

    /// Read one transport frame into `buf`, returning its length.
    fn read_frame(&self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Transport::Udp { sock, addr } => loop {
                let (n, from) = sock.recv_from(buf)?;
                if from.ip() == addr.ip() || n >= 8 {
                    return Ok(n);
                }
            },
            Transport::Tcp { stream } => {
                let mut hdr = [0u8; 8];
                (&**stream).read_exact(&mut hdr)?;
                let n = u16::from_be_bytes([hdr[0], hdr[1]]) as usize;
                if n > buf.len() {
                    return Err(io::Error::other("tcp: buffer too small"));
                }
                (&**stream).read_exact(&mut buf[..n])?;
                Ok(n)
            }
        }
    }
}

pub struct Conn {
    transport: Transport,
    is_tcp: bool,
    seq_ch0: AtomicU16,
    seq_ch3: AtomicU16,
    /// Serializes UDP command write+ack so concurrent writers don't race.
    cmd_mu: Mutex<()>,
    cmd_rx: Receiver<Vec<u8>>,
    packet_rx: Receiver<Vec<u8>>,
    /// Set just before a UDP command write; the worker fires it on msgDrwAck.
    cmd_ack: Arc<Mutex<Option<SyncSender<()>>>>,
    err: Arc<Mutex<Option<String>>>,
}

/// Connect to a CS2 host (UDP control port 32108), optionally forcing
/// `transport` = "udp" | "tcp" (empty = either).
pub fn dial(host: &str, transport: &str) -> Result<Conn> {
    let conn = handshake(host, transport)?;
    let is_tcp = conn.is_tcp();

    // channels: ch0 command (reorder window 10), ch2 packet (window 250).
    let (cmd_tx, cmd_rx) = sync_channel::<Vec<u8>>(100);
    let (packet_tx, packet_rx) = sync_channel::<Vec<u8>>(100);
    let ch0 = DataChannel::new(10);
    let ch2 = DataChannel::new(250);

    let cmd_ack = Arc::new(Mutex::new(None::<SyncSender<()>>));
    let err = Arc::new(Mutex::new(None::<String>));

    let worker = Worker {
        transport: conn.clone(),
        is_tcp,
        ch0: (ch0, cmd_tx),
        ch2: (ch2, packet_tx),
        cmd_ack: Arc::clone(&cmd_ack),
        err: Arc::clone(&err),
    };
    std::thread::spawn(move || worker.run());

    Ok(Conn {
        transport: conn,
        is_tcp,
        seq_ch0: AtomicU16::new(0),
        seq_ch3: AtomicU16::new(0),
        cmd_mu: Mutex::new(()),
        cmd_rx,
        packet_rx,
        cmd_ack,
        err,
    })
}

fn handshake(host: &str, transport: &str) -> Result<Transport> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    let mut addr = resolve_addr(host, 32108)?;
    sock.set_read_timeout(Some(Duration::from_secs(5)))?;

    // LAN search -> wait for punch packet
    let req = [MAGIC, MSG_LAN_SEARCH, 0, 0];
    let res = write_until(&sock, &mut addr, &req, |r| r[1] == MSG_PUNCH_PKT)?;

    let want_udp = transport.is_empty() || transport == "udp";
    let want_tcp = transport.is_empty() || transport == "tcp";

    // echo the punch packet -> wait for "P2P ready"
    let res = write_until(&sock, &mut addr, &res, |r| {
        (want_udp && r[1] == MSG_P2P_RDY_UDP) || (want_tcp && r[1] == MSG_P2P_RDY_TCP)
    })?;

    sock.set_read_timeout(None)?;

    if res[1] == MSG_P2P_RDY_TCP {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
        return Ok(Transport::Tcp {
            stream: Arc::new(stream),
        });
    }

    Ok(Transport::Udp {
        sock: Arc::new(sock),
        addr,
    })
}

fn resolve_addr(host: &str, default_port: u16) -> Result<SocketAddr> {
    if let Ok(mut it) = host.to_socket_addrs() {
        if let Some(a) = it.next() {
            return Ok(a);
        }
    }
    let ip: Ipv4Addr = host
        .parse()
        .map_err(|_| anyhow!("cs2: cannot resolve host {host}"))?;
    Ok(SocketAddr::from((ip, default_port)))
}

/// Repeatedly send `req` (every 1s) until a reply from `addr`'s IP satisfies
/// `ok`, latching the responder's port (CS2 punch behavior).
fn write_until(
    sock: &UdpSocket,
    addr: &mut SocketAddr,
    req: &[u8],
    ok: impl Fn(&[u8]) -> bool,
) -> Result<Vec<u8>> {
    let mut buf = [0u8; 1200];
    let mut last_send = Instant::now() - Duration::from_secs(2);
    loop {
        if last_send.elapsed() >= Duration::from_secs(1) {
            sock.send_to(req, &*addr)?;
            last_send = Instant::now();
        }
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                if from.ip() != addr.ip() || n < 16 {
                    continue;
                }
                if ok(&buf[..n]) {
                    addr.set_port(from.port());
                    return Ok(buf[..n].to_vec());
                }
            }
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                // resend on next loop iteration
            }
            Err(e) => return Err(e.into()),
        }
    }
}

struct Worker {
    transport: Transport,
    is_tcp: bool,
    ch0: (DataChannel, SyncSender<Vec<u8>>),
    ch2: (DataChannel, SyncSender<Vec<u8>>),
    cmd_ack: Arc<Mutex<Option<SyncSender<()>>>>,
    err: Arc<Mutex<Option<String>>>,
}

impl Worker {
    fn run(mut self) {
        let mut keepalive = Instant::now();
        let mut buf = [0u8; 1200];
        loop {
            let n = match self.transport.read_frame(&mut buf) {
                Ok(n) if n >= 2 => n,
                Ok(_) => continue,
                Err(e) => {
                    *self.err.lock().unwrap() = Some(format!("cs2: {e}"));
                    return;
                }
            };
            let frame = &buf[..n];
            match frame[1] {
                MSG_DRW => {
                    let ch = frame[5];
                    if self.is_tcp {
                        // TCP: ping ~every second to keep alive.
                        if Instant::now() >= keepalive {
                            let _ = self.transport.write(&[MAGIC, MSG_PING, 0, 0]);
                            keepalive = Instant::now() + Duration::from_secs(1);
                        }
                        if let Err(e) = self.push(ch, &frame[8..n]) {
                            *self.err.lock().unwrap() = Some(format!("cs2: {e}"));
                            return;
                        }
                    } else {
                        let (seq_hi, seq_lo) = (frame[6], frame[7]);
                        let seq = u16::from_be_bytes([seq_hi, seq_lo]);
                        match self.push_seq(ch, seq, &frame[8..n]) {
                            Ok(pushed) => {
                                if pushed >= 0 {
                                    let ack = [
                                        MAGIC,
                                        MSG_DRW_ACK,
                                        0,
                                        6,
                                        MAGIC_DRW,
                                        ch,
                                        0,
                                        1,
                                        seq_hi,
                                        seq_lo,
                                    ];
                                    let _ = self.transport.write(&ack);
                                }
                            }
                            Err(e) => {
                                *self.err.lock().unwrap() = Some(format!("cs2: {e}"));
                                return;
                            }
                        }
                    }
                }
                MSG_PING => {
                    let _ = self.transport.write(&[MAGIC, MSG_PONG, 0, 0]);
                }
                MSG_DRW_ACK => {
                    if let Some(tx) = self.cmd_ack.lock().unwrap().as_ref() {
                        let _ = tx.try_send(());
                    }
                }
                // pong / p2p-ready / close / close-ack: ignore
                MSG_PONG | MSG_P2P_RDY_UDP | MSG_P2P_RDY_TCP | 0xF0 | 0xF1 => {}
                other => {
                    log::debug!("cs2: unknown msg 0x{other:02x}");
                }
            }
        }
    }

    fn channel_mut(&mut self, ch: u8) -> Option<(&mut DataChannel, &SyncSender<Vec<u8>>)> {
        match ch {
            0 => Some((&mut self.ch0.0, &self.ch0.1)),
            2 => Some((&mut self.ch2.0, &self.ch2.1)),
            _ => None,
        }
    }

    fn push(&mut self, ch: u8, data: &[u8]) -> Result<()> {
        if let Some((channel, tx)) = self.channel_mut(ch) {
            channel.push(data, tx)?;
        }
        Ok(())
    }

    fn push_seq(&mut self, ch: u8, seq: u16, data: &[u8]) -> Result<i32> {
        if let Some((channel, tx)) = self.channel_mut(ch) {
            channel.push_seq(seq, data, tx)
        } else {
            Ok(-1)
        }
    }
}

impl Conn {
    pub fn protocol(&self) -> &'static str {
        if self.is_tcp { "cs2+tcp" } else { "cs2+udp" }
    }

    fn error(&self) -> anyhow::Error {
        match &*self.err.lock().unwrap() {
            Some(e) => anyhow!(e.clone()),
            None => anyhow!("cs2: closed"),
        }
    }

    /// Read a command frame from channel 0: `(cmd, data)`.
    pub fn read_command(&self) -> Result<(u32, Vec<u8>)> {
        let buf = self.cmd_rx.recv().map_err(|_| self.error())?;
        if buf.len() < 4 {
            bail!("cs2: short command");
        }
        let cmd = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        Ok((cmd, buf[4..].to_vec()))
    }

    /// Write a command on channel 0. For UDP, retransmits until ACK (5x, 1s).
    pub fn write_command(&self, cmd: u32, data: &[u8]) -> Result<()> {
        let _guard = self.cmd_mu.lock().unwrap();
        let seq = self.seq_ch0.fetch_add(1, Ordering::Relaxed);
        let req = marshal_cmd(0, seq, cmd, data);

        if self.is_tcp {
            return self.transport.write(&req).map_err(Into::into);
        }

        let (ack_tx, ack_rx) = sync_channel::<()>(1);
        *self.cmd_ack.lock().unwrap() = Some(ack_tx);

        let mut repeat = 5;
        loop {
            self.transport.write(&req)?;
            match ack_rx.recv_timeout(Duration::from_secs(1)) {
                Ok(()) => {
                    *self.cmd_ack.lock().unwrap() = None;
                    return Ok(());
                }
                Err(_) => {
                    repeat -= 1;
                    if repeat <= 0 {
                        *self.cmd_ack.lock().unwrap() = None;
                        bail!("cs2: can't send command {cmd}");
                    }
                }
            }
        }
    }

    /// Read a media packet from channel 2: `(header[..32], payload)`.
    pub fn read_packet(&self) -> Result<(Vec<u8>, Vec<u8>)> {
        let data = self.packet_rx.recv().map_err(|_| self.error())?;
        if data.len() < HDR_SIZE {
            bail!("cs2: short packet");
        }
        Ok((data[..HDR_SIZE].to_vec(), data[HDR_SIZE..].to_vec()))
    }

    /// Write a media packet on channel 3 (used by the backchannel).
    pub fn write_packet(&self, hdr: &[u8], payload: &[u8]) -> Result<()> {
        const OFFSET: usize = 12;
        let n = (HDR_SIZE + payload.len()) as u32;
        let mut req = vec![0u8; n as usize + OFFSET];
        req[0] = MAGIC;
        req[1] = MSG_DRW;
        req[2..4].copy_from_slice(&((n + 8) as u16).to_be_bytes());
        req[4] = MAGIC_DRW;
        req[5] = 3; // channel
        let seq = self.seq_ch3.fetch_add(1, Ordering::Relaxed);
        req[6..8].copy_from_slice(&seq.to_be_bytes());
        req[8..12].copy_from_slice(&n.to_be_bytes());
        let hlen = hdr.len().min(HDR_SIZE);
        req[OFFSET..OFFSET + hlen].copy_from_slice(&hdr[..hlen]);
        // NOTE: matches the Go source, which copies `hdr` into the payload slot too.
        let plen = payload.len().min(req.len() - (OFFSET + HDR_SIZE));
        req[OFFSET + HDR_SIZE..OFFSET + HDR_SIZE + plen].copy_from_slice(&payload[..plen]);
        self.transport.write(&req).map_err(Into::into)
    }
}

fn marshal_cmd(channel: u8, seq: u16, cmd: u32, payload: &[u8]) -> Vec<u8> {
    let size = payload.len();
    let mut req = vec![0u8; 16 + size];
    // 1. message header
    req[0] = MAGIC;
    req[1] = MSG_DRW;
    req[2..4].copy_from_slice(&((4 + 4 + 4 + size) as u16).to_be_bytes());
    // 2. drw header
    req[4] = MAGIC_DRW;
    req[5] = channel;
    req[6..8].copy_from_slice(&seq.to_be_bytes());
    // 3. payload size
    req[8..12].copy_from_slice(&((4 + size) as u32).to_be_bytes());
    // 4. command
    req[12..16].copy_from_slice(&cmd.to_be_bytes());
    // 5. payload
    req[16..].copy_from_slice(payload);
    req
}

/// Reassembles length-prefixed records out of a (possibly seq-reordered) stream.
struct DataChannel {
    wait_seq: u16,
    push_buf: HashMap<u16, Vec<u8>>,
    push_size: usize,
    wait_data: Vec<u8>,
    wait_size: usize,
}

impl DataChannel {
    fn new(push_size: usize) -> Self {
        Self {
            wait_seq: 0,
            push_buf: HashMap::new(),
            push_size,
            wait_data: Vec::new(),
            wait_size: 0,
        }
    }

    fn push(&mut self, b: &[u8], tx: &SyncSender<Vec<u8>>) -> Result<()> {
        self.wait_data.extend_from_slice(b);
        while self.wait_data.len() > 4 {
            if self.wait_size == 0 {
                self.wait_size = u32::from_be_bytes([
                    self.wait_data[0],
                    self.wait_data[1],
                    self.wait_data[2],
                    self.wait_data[3],
                ]) as usize;
                self.wait_data.drain(..4);
            }
            if self.wait_size > self.wait_data.len() {
                break;
            }
            let record: Vec<u8> = self.wait_data.drain(..self.wait_size).collect();
            match tx.try_send(record) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => bail!("pop buffer is full"),
                Err(TrySendError::Disconnected(_)) => bail!("pop buffer closed"),
            }
            self.wait_size = 0;
        }
        Ok(())
    }

    /// Returns how many seqs were processed; 0 if buffered/old, -1 if undeliverable.
    fn push_seq(&mut self, seq: u16, data: &[u8], tx: &SyncSender<Vec<u8>>) -> Result<i32> {
        let diff = seq.wrapping_sub(self.wait_seq) as i16;
        if diff > 0 {
            if self.push_size == 0 {
                return Ok(-1);
            }
            if !self.push_buf.contains_key(&seq) {
                if self.push_buf.len() == self.push_size {
                    return Ok(-1);
                }
                self.push_buf.insert(seq, data.to_vec());
            }
            return Ok(0);
        }
        if diff < 0 {
            return Ok(0);
        }

        let mut cur = data.to_vec();
        let mut i = 1;
        loop {
            self.push(&cur, tx)?;
            self.wait_seq = self.wait_seq.wrapping_add(1);
            match self.push_buf.remove(&self.wait_seq) {
                Some(next) => cur = next,
                None => return Ok(i),
            }
            i += 1;
        }
    }
}

// Keep a per-process frame counter available for debugging/metrics.
#[allow(dead_code)]
static FRAMES: AtomicU64 = AtomicU64::new(0);
#[allow(dead_code)]
fn note_frame() {
    FRAMES.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marshal_cmd_layout() {
        let req = marshal_cmd(0, 0x0102, 0xAABBCCDD, &[1, 2, 3]);
        assert_eq!(req[0], MAGIC);
        assert_eq!(req[1], MSG_DRW);
        assert_eq!(&req[2..4], &(15u16).to_be_bytes()); // 4+4+4+3
        assert_eq!(req[4], MAGIC_DRW);
        assert_eq!(&req[6..8], &[0x01, 0x02]);
        assert_eq!(&req[8..12], &(7u32).to_be_bytes()); // 4+3
        assert_eq!(&req[12..16], &0xAABBCCDDu32.to_be_bytes());
        assert_eq!(&req[16..], &[1, 2, 3]);
    }

    #[test]
    fn datachannel_reassembles_length_prefixed_records() {
        let (tx, rx) = sync_channel::<Vec<u8>>(8);
        let mut ch = DataChannel::new(10);
        // one record "hello" split across two pushes
        ch.push(&[0, 0, 0, 5, b'h', b'e'], &tx).unwrap();
        ch.push(&[b'l', b'l', b'o'], &tx).unwrap();
        assert_eq!(rx.recv().unwrap(), b"hello");
    }

    #[test]
    fn datachannel_reorders_by_seq() {
        let (tx, rx) = sync_channel::<Vec<u8>>(8);
        let mut ch = DataChannel::new(10);
        // seq 1 arrives before seq 0 -> buffered, then flushed in order
        assert_eq!(ch.push_seq(1, &[0, 0, 0, 1, b'B'], &tx).unwrap(), 0);
        assert_eq!(ch.push_seq(0, &[0, 0, 0, 1, b'A'], &tx).unwrap(), 2);
        assert_eq!(rx.recv().unwrap(), b"A");
        assert_eq!(rx.recv().unwrap(), b"B");
    }
}
