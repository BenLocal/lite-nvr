# GB28181 P4a — Transport + ZLM Control Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the P1-3 UDP-only gb bridge into a full UDP/TCP-passive/TCP-active on-demand puller, driven by a dedicated ZLM control worker thread and a per-request `POST /gb/play` endpoint.

**Architecture:** A dedicated std::thread (started by `ZlmControl::spawn`) owns all `RtpServer`s keyed by `stream_id` and runs ZLM's synchronous control FFI (`RtpServer` create/connect/close, `rtp_get_info`) via `handler_zlm_cmd`, behind a tokio-mpsc command channel with tokio-oneshot replies. The `MediaReceiver`/`ReceiverHandle` seam is reworked over `ZlmControl` (async, `unsafe impl Send` deleted); `start_pull` reads the transport from the stream mapping and does the active-mode two-phase open→INVITE→connect.

**Tech Stack:** Rust (edition 2024), tokio, axum, `async-trait`, rszlm ≥ `0e1af4d` (v0.2.0), gb28181 crate.

**Spec:** `docs/superpowers/specs/2026-07-02-gb28181-zlm-control-layer-design.md` (this is P4a = §8 "transport + control core"; the media-state layer / `GET /gb/streams` is P4b).

---

## Context the implementer needs (read before starting)

**Build/test env (critical — every cargo command):**
```bash
export CARGO_TARGET_DIR=/root/workspace/master/lite-nvr/target-gb28181
export LD_LIBRARY_PATH="$PWD/ffmpeg/lib:$PWD/zlm/lib:$LD_LIBRARY_PATH"
export ZLM_DIR="$(ls -d $PWD/target*/debug/build/rszlm-sys-*/out/zlm-install 2>/dev/null | head -1)"
```
- Use `cargo check -p nvr` (NOT `cargo clippy -p nvr` — a pre-existing ffmpeg-bus `never_loop` clippy error blocks it).
- nvr gb unit tests: `cargo test -p nvr --bin nvr gb::` (the nvr test target has `harness`/`test=false` quirks; scope to `gb::`). The gb bridge tests use a **fake receiver** and do not call ZLM, so they pass without a running ZLM (the nvr binary still dynamically links `libmk_api.so`, hence `LD_LIBRARY_PATH`).
- `cargo fmt` before every commit.

**Current code (verified facts):**
- `stream_id` **is the nvr device id**. Mappings are registered at startup (`nvr/src/init/device.rs:79`, `bridge.register_mapping(&device.id, &gb.device_id, &gb.channel_id)`) for `input_type=gb28181` devices; removed on device update/delete (`nvr/src/handler/device.rs`).
- gb streams publish under ZLM's `rtp` app; play URL = `crate::init::device::build_gb_flv_url(device_id)` (`pub(crate)`), returning `http://127.0.0.1:8553/rtp/{device_id}.live.flv`.
- `gb28181::Transport` = `Udp | TcpPassive | TcpActive` (`#[derive(Debug, Clone, Copy, PartialEq, Eq)]`, **no serde**).
- `GbServer::invite_play(device_id, channel_id, spec) -> Result<MediaSession>` sets `spec.negotiated_remote = Some(answer.media_addr)` then returns a `MediaSession` whose `spec` field is **public** — so the active-connect address is `session.spec.negotiated_remote`.
- `MediaSpec { ssrc, ssrc_str, transport, media_addr, stream_type, negotiated_remote }`.
- The repo does **not** use `async_trait` yet; native async-fn-in-trait is not `dyn`-safe, so this plan adds the `async-trait` crate for the `dyn MediaReceiver`/`dyn ReceiverHandle` seam.
- `rszlm` API (checkout `0e1af4d`): `rszlm::server::RtpServer::new(port: u16, mode: RtpServerTcpMode, stream_id: &str)`; `RtpServerTcpMode::{Disabled=0, Passive=1, Active=2}`; `RtpServer::bind_port() -> u16`; `RtpServer::connect(url: &str, dst_port: u16, cb: impl FnMut(i32, String, i32) + Send + Sync + 'static)`; `rszlm::server::rtp_get_info(app: &str, stream: &str) -> Option<RtpInfo>`. Dropping an `RtpServer` releases its port.

**Existing scaffold to extend (do NOT create a parallel module):** `nvr/src/zlm/cmd.rs` already has an empty `pub enum ZlmCmd {}`, a global `ZLM_CMD_SENDER` `OnceLock`, `init_zlm_cmd_sender`, `blocking_send_cmd`, and a stub `handler_zlm_cmd(_cmd) -> anyhow::Result<()>`. Task 1 replaces the global-sender scaffold with the injectable `ZlmControl` facade + a real `handler_zlm_cmd(cmd, &mut servers)`. `nvr/src/zlm/mod.rs` already declares `pub mod cmd;`.

---

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `nvr/src/zlm/cmd.rs` | rewrite | `ZlmCmd` (4 variants + oneshot replies), `ZlmControl` facade, worker thread, `handler_zlm_cmd`, `mode_for` |
| `nvr/src/zlm/cmd_test.rs` | create | unit-test `mode_for` + `ZlmControl` command dispatch (facade, no ZLM) |
| `nvr/Cargo.toml` | modify | add `async-trait` |
| `nvr/src/gb/stream_map.rs` | modify | `Mapping.transport`, `register(.., transport)`, `set_transport`, `list` |
| `nvr/src/gb/receiver.rs` | rewrite | async `MediaReceiver`/`ReceiverHandle` over `ZlmControl`; delete `unsafe impl Send`; fake records transport + connect |
| `nvr/src/gb/bridge.rs` | modify | transport-aware two-phase `start_pull` |
| `nvr/src/gb/bridge_test.rs` | modify | assert transport threading + active two-phase |
| `nvr/src/gb/mod.rs` | modify | spawn `ZlmControl`, build `ZlmRtpReceiver::new(control)` |
| `nvr/src/gb/api.rs` | modify | `POST /gb/play` |
| `nvr/src/init/device.rs` | modify | pass `Transport::Udp` to `register_mapping` (signature change) |

---

## Task 1: ZlmControl worker + ZlmCmd + handler_zlm_cmd

**Files:**
- Modify (rewrite): `nvr/src/zlm/cmd.rs`
- Test: `nvr/src/zlm/cmd_test.rs` (create)

- [ ] **Step 1: Write the failing test** — create `nvr/src/zlm/cmd_test.rs`:

```rust
use super::*;
use gb28181::Transport;

#[test]
fn mode_for_maps_transports() {
    assert!(matches!(mode_for(Transport::Udp), rszlm::server::RtpServerTcpMode::Disabled));
    assert!(matches!(mode_for(Transport::TcpPassive), rszlm::server::RtpServerTcpMode::Passive));
    assert!(matches!(mode_for(Transport::TcpActive), rszlm::server::RtpServerTcpMode::Active));
}

// The facade builds the right command and returns the worker's reply, without
// touching ZLM: we stand in for the worker by draining the channel ourselves.
#[tokio::test]
async fn open_rtp_sends_command_and_returns_reply() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ZlmCmd>(4);
    let ctrl = ZlmControl::for_test(tx);
    let task = tokio::spawn(async move { ctrl.open_rtp("cam1", rszlm::server::RtpServerTcpMode::Disabled).await });

    match rx.recv().await.expect("cmd") {
        ZlmCmd::OpenRtp { stream_id, reply, .. } => {
            assert_eq!(stream_id, "cam1");
            reply.send(Ok(41000)).unwrap();
        }
        other => panic!("unexpected cmd: {other:?}"),
    }
    assert_eq!(task.await.unwrap().unwrap(), 41000);
}

#[tokio::test]
async fn rtp_info_sends_command_and_returns_reply() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ZlmCmd>(4);
    let ctrl = ZlmControl::for_test(tx);
    let task = tokio::spawn(async move { ctrl.rtp_info("rtp", "cam1").await });
    match rx.recv().await.expect("cmd") {
        ZlmCmd::GetRtpInfo { app, stream, reply } => {
            assert_eq!((app.as_str(), stream.as_str()), ("rtp", "cam1"));
            reply.send(None).unwrap();
        }
        other => panic!("unexpected cmd: {other:?}"),
    }
    assert!(task.await.unwrap().is_none());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nvr --bin nvr zlm::cmd 2>&1 | tail -20`
Expected: FAIL — `mode_for`, `ZlmControl::for_test`, `ZlmCmd` variants not defined.

- [ ] **Step 3: Write the implementation** — replace the entire contents of `nvr/src/zlm/cmd.rs`:

```rust
//! The ZLM control worker: a dedicated OS thread owns every `RtpServer` (keyed
//! by stream id) and runs ZLM's synchronous control FFI. Async callers use the
//! `ZlmControl` facade (a cheap-clone tokio-mpsc sender) and await a oneshot
//! reply. Keeping the `RtpServer`s on one thread means they never cross a thread
//! boundary — no `unsafe impl Send` anywhere.

use std::collections::HashMap;
use std::net::SocketAddr;

use gb28181::Transport;
use rszlm::server::{RtpInfo, RtpServer, RtpServerTcpMode};
use tokio::sync::{mpsc, oneshot};

/// A command for the ZLM worker thread. Variants that produce a result carry a
/// oneshot `reply`.
pub enum ZlmCmd {
    /// Create an `RtpServer` in `mode` for `stream_id`; reply with the bound port.
    OpenRtp {
        stream_id: String,
        mode: RtpServerTcpMode,
        reply: oneshot::Sender<anyhow::Result<u16>>,
    },
    /// TCP-active: connect the existing server for `stream_id` out to `remote`.
    ConnectRtp {
        stream_id: String,
        remote: SocketAddr,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    /// Drop the `RtpServer` for `stream_id` (releases the port). Fire-and-forget.
    CloseRtp { stream_id: String },
    /// Query live RTP receive info for `app`/`stream`.
    GetRtpInfo {
        app: String,
        stream: String,
        reply: oneshot::Sender<Option<RtpInfo>>,
    },
}

impl std::fmt::Debug for ZlmCmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZlmCmd::OpenRtp { stream_id, .. } => write!(f, "OpenRtp({stream_id})"),
            ZlmCmd::ConnectRtp { stream_id, remote, .. } => write!(f, "ConnectRtp({stream_id},{remote})"),
            ZlmCmd::CloseRtp { stream_id } => write!(f, "CloseRtp({stream_id})"),
            ZlmCmd::GetRtpInfo { app, stream, .. } => write!(f, "GetRtpInfo({app},{stream})"),
        }
    }
}

/// Map a gb transport to ZLM's RTP server TCP mode.
pub fn mode_for(transport: Transport) -> RtpServerTcpMode {
    match transport {
        Transport::Udp => RtpServerTcpMode::Disabled,
        Transport::TcpPassive => RtpServerTcpMode::Passive,
        Transport::TcpActive => RtpServerTcpMode::Active,
    }
}

/// Cheap-clone async facade over the worker's command channel.
#[derive(Clone)]
pub struct ZlmControl {
    tx: mpsc::Sender<ZlmCmd>,
}

impl ZlmControl {
    /// Start the worker thread and return the facade.
    pub fn spawn() -> Self {
        let (tx, mut rx) = mpsc::channel::<ZlmCmd>(1024);
        std::thread::Builder::new()
            .name("zlm-rtp".into())
            .spawn(move || {
                let mut servers: HashMap<String, RtpServer> = HashMap::new();
                // `blocking_recv` drains the tokio channel from a plain OS thread.
                while let Some(cmd) = rx.blocking_recv() {
                    handler_zlm_cmd(cmd, &mut servers);
                }
            })
            .expect("spawn zlm-rtp worker thread");
        Self { tx }
    }

    #[cfg(test)]
    pub fn for_test(tx: mpsc::Sender<ZlmCmd>) -> Self {
        Self { tx }
    }

    pub async fn open_rtp(&self, stream_id: &str, mode: RtpServerTcpMode) -> anyhow::Result<u16> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ZlmCmd::OpenRtp { stream_id: stream_id.to_string(), mode, reply })
            .await
            .map_err(|_| anyhow::anyhow!("zlm worker gone"))?;
        rx.await.map_err(|_| anyhow::anyhow!("zlm worker dropped reply"))?
    }

    pub async fn connect_rtp(&self, stream_id: &str, remote: SocketAddr) -> anyhow::Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ZlmCmd::ConnectRtp { stream_id: stream_id.to_string(), remote, reply })
            .await
            .map_err(|_| anyhow::anyhow!("zlm worker gone"))?;
        rx.await.map_err(|_| anyhow::anyhow!("zlm worker dropped reply"))?
    }

    /// Fire-and-forget: safe to call from a `Drop` (never blocks / awaits).
    pub fn close_rtp(&self, stream_id: &str) {
        if let Err(e) = self.tx.try_send(ZlmCmd::CloseRtp { stream_id: stream_id.to_string() }) {
            log::warn!("gb28181: close_rtp({stream_id}) not queued: {e}");
        }
    }

    pub async fn rtp_info(&self, app: &str, stream: &str) -> Option<RtpInfo> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(ZlmCmd::GetRtpInfo { app: app.to_string(), stream: stream.to_string(), reply })
            .await
            .is_err()
        {
            return None;
        }
        rx.await.ok().flatten()
    }
}

/// Execute one command against the worker's `RtpServer` table. Runs only on the
/// worker thread.
pub(crate) fn handler_zlm_cmd(cmd: ZlmCmd, servers: &mut HashMap<String, RtpServer>) {
    match cmd {
        ZlmCmd::OpenRtp { stream_id, mode, reply } => {
            // port 0 = let ZLM pick a free port; bind_port() reports it.
            let server = RtpServer::new(0, mode, &stream_id);
            let port = server.bind_port();
            servers.insert(stream_id, server);
            let _ = reply.send(if port == 0 {
                Err(anyhow::anyhow!("rtp server failed to bind a port"))
            } else {
                Ok(port)
            });
        }
        ZlmCmd::ConnectRtp { stream_id, remote, reply } => {
            let Some(server) = servers.get(&stream_id) else {
                let _ = reply.send(Err(anyhow::anyhow!("connect: no rtp server for {stream_id}")));
                return;
            };
            // The connect result arrives asynchronously on a ZLM thread via this
            // callback (FnMut) — move the reply in and fire it once.
            let mut reply = Some(reply);
            server.connect(&remote.ip().to_string(), remote.port(), move |code, msg, _| {
                if let Some(reply) = reply.take() {
                    let _ = reply.send(if code == 0 {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("rtp connect failed: code={code} {msg}"))
                    });
                }
            });
        }
        ZlmCmd::CloseRtp { stream_id } => {
            servers.remove(&stream_id); // Drop releases the port.
        }
        ZlmCmd::GetRtpInfo { app, stream, reply } => {
            let _ = reply.send(rszlm::server::rtp_get_info(&app, &stream));
        }
    }
}

#[cfg(test)]
#[path = "cmd_test.rs"]
mod cmd_test;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nvr --bin nvr zlm::cmd 2>&1 | tail -20`
Expected: PASS — `mode_for_maps_transports`, `open_rtp_sends_command_and_returns_reply`, `rtp_info_sends_command_and_returns_reply`.

- [ ] **Step 5: Verify the whole crate still builds + fmt**

Run: `cargo fmt && cargo check -p nvr 2>&1 | tail -5`
Expected: `Finished` (the old global-sender helpers are gone; confirm nothing referenced them — they were `#[allow(dead_code)]` and unused).

- [ ] **Step 6: Commit**

```bash
git add nvr/src/zlm/cmd.rs nvr/src/zlm/cmd_test.rs
git commit -m "feat(gb28181): ZLM control worker (ZlmControl + ZlmCmd + handler_zlm_cmd)"
```

---

## Task 2: Mapping gains transport

**Files:**
- Modify: `nvr/src/gb/stream_map.rs`
- Modify: `nvr/src/gb/bridge.rs:41` (`register_mapping` signature)
- Modify: `nvr/src/init/device.rs:79` (pass `Transport::Udp`)

- [ ] **Step 1: Write the failing test** — in `nvr/src/gb/stream_map.rs`, add to `mod tests`:

```rust
#[test]
fn register_stores_transport_and_set_transport_updates() {
    use gb28181::Transport;
    let m = StreamMap::new();
    m.register("cam1", "d1", "c1", Transport::Udp);
    assert_eq!(m.get("cam1").unwrap().transport, Transport::Udp);
    assert!(m.set_transport("cam1", Transport::TcpActive));
    assert_eq!(m.get("cam1").unwrap().transport, Transport::TcpActive);
    assert!(!m.set_transport("nope", Transport::TcpActive));
}

#[test]
fn list_returns_all_mappings() {
    use gb28181::Transport;
    let m = StreamMap::new();
    m.register("cam1", "d1", "c1", Transport::Udp);
    m.register("cam2", "d2", "c2", Transport::TcpPassive);
    let mut ids: Vec<String> = m.list().into_iter().map(|(id, _)| id).collect();
    ids.sort();
    assert_eq!(ids, vec!["cam1".to_string(), "cam2".to_string()]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nvr --bin nvr gb::stream_map 2>&1 | tail -20`
Expected: FAIL — `register` takes 3 args, `set_transport`/`list`/`Mapping.transport` missing.

- [ ] **Step 3: Implement** — update `nvr/src/gb/stream_map.rs`:

Add `use gb28181::Transport;` at the top. Change `Mapping`:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mapping {
    pub device_id: String,
    pub channel_id: String,
    pub transport: Transport,
}
```
Change `register` and add `set_transport` + `list`:
```rust
    /// Insert or overwrite the mapping for `stream_id`.
    pub fn register(&self, stream_id: &str, device_id: &str, channel_id: &str, transport: Transport) {
        self.inner.lock().unwrap().insert(
            stream_id.to_string(),
            Mapping {
                device_id: device_id.to_string(),
                channel_id: channel_id.to_string(),
                transport,
            },
        );
    }

    /// Update the transport of an existing mapping. Returns false if absent.
    pub fn set_transport(&self, stream_id: &str, transport: Transport) -> bool {
        match self.inner.lock().unwrap().get_mut(stream_id) {
            Some(m) => {
                m.transport = transport;
                true
            }
            None => false,
        }
    }

    /// Snapshot of all (stream_id, mapping) pairs.
    pub fn list(&self) -> Vec<(String, Mapping)> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
```
Fix the two existing `mod tests` cases that call `register(...)`/build `Mapping {...}` to include `transport: Transport::Udp` (the `register_get_unregister_roundtrip` and `register_overwrites` tests, and their `Mapping { .. }` literals).

Then update the two production callers:
- `nvr/src/gb/bridge.rs:41` — change signature + body:
```rust
    pub fn register_mapping(&self, stream_id: &str, device_id: &str, channel_id: &str, transport: gb28181::Transport) {
        self.streams.register(stream_id, device_id, channel_id, transport);
    }
```
- `nvr/src/init/device.rs:79` — pass the default:
```rust
                bridge.register_mapping(&device.id, &gb.device_id, &gb.channel_id, gb28181::Transport::Udp);
```
- `nvr/src/gb/bridge_test.rs` — the existing `register_mapping("cam1", DEVICE, CHANNEL)` calls (two of them, ~lines 67 and 86) now need the transport arg:
```rust
    bridge.register_mapping("cam1", DEVICE, CHANNEL, gb28181::Transport::Udp);
```

- [ ] **Step 4: Run tests + build**

Run: `cargo test -p nvr --bin nvr gb::stream_map 2>&1 | tail -20 && cargo check -p nvr 2>&1 | tail -5`
Expected: stream_map tests PASS; crate `Finished`.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add nvr/src/gb/stream_map.rs nvr/src/gb/bridge.rs nvr/src/init/device.rs
git commit -m "feat(gb28181): stream mapping carries transport (default udp)"
```

---

## Task 3: Async receiver seam over ZlmControl (delete unsafe Send) + wiring

**Files:**
- Modify: `nvr/Cargo.toml` (add `async-trait`)
- Modify (rewrite): `nvr/src/gb/receiver.rs`
- Modify: `nvr/src/gb/bridge.rs` (`start_pull` await; still Udp here)
- Modify: `nvr/src/gb/mod.rs` (spawn `ZlmControl`, build receiver from it)

This is the seam swap; it must land together to compile. `start_pull` stays UDP in this task (Task 4 makes it transport-aware).

- [ ] **Step 1: Add the dependency** — in `nvr/Cargo.toml` `[dependencies]` add:
```toml
async-trait = "0.1"
```
Run: `cargo fetch 2>&1 | tail -3` (Expected: resolves `async-trait`).

- [ ] **Step 2: Rewrite `nvr/src/gb/receiver.rs`:**

```rust
//! The media-receive seam. The bridge depends only on the `MediaReceiver` trait,
//! so pull/teardown logic is testable without ZLM. The real impl drives the
//! `ZlmControl` worker; the worker owns the `RtpServer`, so nothing here holds a
//! raw ZLM pointer — no `unsafe impl Send`.

use std::net::SocketAddr;

use async_trait::async_trait;
use gb28181::Transport;

use crate::zlm::cmd::{mode_for, ZlmControl};

/// A live receiver ZLM opened for one stream. Dropping it releases the port.
#[async_trait]
pub trait ReceiverHandle: Send {
    /// Port the device must send its PS/RTP to (UDP) or connect to (TCP passive).
    fn port(&self) -> u16;
    /// TCP-active only: connect out to the device's media addr (from the SDP
    /// answer). No-op / unused for UDP and TCP-passive.
    async fn connect(&self, remote: SocketAddr) -> anyhow::Result<()>;
}

/// Opens a media receiver for a stream id.
#[async_trait]
pub trait MediaReceiver: Send + Sync {
    async fn open(
        &self,
        stream_id: &str,
        transport: Transport,
    ) -> anyhow::Result<Box<dyn ReceiverHandle>>;
}

/// Real receiver: drives the `ZlmControl` worker to create/connect/close
/// `RtpServer`s that publish `stream_id` under ZLM's `rtp` app.
pub struct ZlmRtpReceiver {
    control: ZlmControl,
}

impl ZlmRtpReceiver {
    pub fn new(control: ZlmControl) -> Self {
        Self { control }
    }
}

struct ZlmReceiverHandle {
    stream_id: String,
    port: u16,
    control: ZlmControl,
}

#[async_trait]
impl ReceiverHandle for ZlmReceiverHandle {
    fn port(&self) -> u16 {
        self.port
    }
    async fn connect(&self, remote: SocketAddr) -> anyhow::Result<()> {
        self.control.connect_rtp(&self.stream_id, remote).await
    }
}

impl Drop for ZlmReceiverHandle {
    fn drop(&mut self) {
        self.control.close_rtp(&self.stream_id); // fire-and-forget, releases the port
    }
}

#[async_trait]
impl MediaReceiver for ZlmRtpReceiver {
    async fn open(
        &self,
        stream_id: &str,
        transport: Transport,
    ) -> anyhow::Result<Box<dyn ReceiverHandle>> {
        let port = self.control.open_rtp(stream_id, mode_for(transport)).await?;
        Ok(Box::new(ZlmReceiverHandle {
            stream_id: stream_id.to_string(),
            port,
            control: self.control.clone(),
        }))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    use std::sync::Arc;

    /// Test receiver: hands out deterministic ports and records (stream_id,
    /// transport) per open and the remote of each active connect. All state is
    /// behind `Arc` so a test can `clone()` the receiver, move one copy into the
    /// bridge, and read the recordings through the other.
    #[derive(Default, Clone)]
    pub struct FakeReceiver {
        pub opened: Arc<Mutex<Vec<(String, Transport)>>>,
        pub connected: Arc<Mutex<Vec<SocketAddr>>>,
        next_port: Arc<Mutex<u16>>,
    }

    pub struct FakeHandle {
        port: u16,
        connected: Arc<Mutex<Vec<SocketAddr>>>,
    }

    #[async_trait]
    impl ReceiverHandle for FakeHandle {
        fn port(&self) -> u16 {
            self.port
        }
        async fn connect(&self, remote: SocketAddr) -> anyhow::Result<()> {
            self.connected.lock().unwrap().push(remote);
            Ok(())
        }
    }

    #[async_trait]
    impl MediaReceiver for FakeReceiver {
        async fn open(
            &self,
            stream_id: &str,
            transport: Transport,
        ) -> anyhow::Result<Box<dyn ReceiverHandle>> {
            self.opened.lock().unwrap().push((stream_id.to_string(), transport));
            let mut p = self.next_port.lock().unwrap();
            *p = if *p == 0 { 40000 } else { *p + 2 };
            Ok(Box::new(FakeHandle { port: *p, connected: self.connected.clone() }))
        }
    }

    #[tokio::test]
    async fn fake_records_open_and_connect() {
        let r = FakeReceiver::default();
        let h = r.open("cam1", Transport::TcpActive).await.unwrap();
        assert_eq!(h.port(), 40000);
        h.connect("1.2.3.4:5000".parse().unwrap()).await.unwrap();
        assert_eq!(r.opened.lock().unwrap().as_slice(), &[("cam1".to_string(), Transport::TcpActive)]);
        assert_eq!(r.connected.lock().unwrap().as_slice(), &["1.2.3.4:5000".parse().unwrap()]);
    }
}
```

- [ ] **Step 3: Update `nvr/src/gb/bridge.rs::start_pull`** — make the open call await (keep Udp for now). Change the first line of `start_pull`'s body:
```rust
        let handle = self.receiver.open(stream_id, Transport::Udp).await?;
```
(The rest of `start_pull` is unchanged in this task.)

- [ ] **Step 4: Wire `ZlmControl` in `nvr/src/gb/mod.rs`** — add `use crate::zlm::cmd::ZlmControl;` and change the bridge construction (around line 41):
```rust
    let control = ZlmControl::spawn();
    let bridge = Arc::new(GbBridge::new(
        server,
        cfg.media_ip.clone(),
        Box::new(ZlmRtpReceiver::new(control)),
    ));
```

- [ ] **Step 5: Build + run gb tests**

Run: `cargo fmt && cargo check -p nvr 2>&1 | tail -5 && cargo test -p nvr --bin nvr gb:: 2>&1 | tail -15`
Expected: crate `Finished`; existing gb tests + `fake_records_open_and_connect` PASS. (Existing `bridge_test` cases still compile — they build `FakeReceiver::default()` and call `handle_media_not_found`, both still valid; `open` being async is internal to the bridge.)

- [ ] **Step 6: Commit**

```bash
git add nvr/Cargo.toml Cargo.lock nvr/src/gb/receiver.rs nvr/src/gb/bridge.rs nvr/src/gb/mod.rs
git commit -m "refactor(gb28181): receiver seam over ZlmControl worker (drop unsafe Send)"
```

---

## Task 4: Transport-aware two-phase start_pull

**Files:**
- Modify: `nvr/src/gb/bridge.rs` (`start_pull`)
- Modify: `nvr/src/gb/bridge_test.rs` (assert transport threading + active connect)

- [ ] **Step 1: Write the failing tests** — in `nvr/src/gb/bridge_test.rs`, add these two cases. They reuse the module's existing `PLATFORM/DOMAIN/DEVICE/CHANNEL` constants, `spawn_answering_client` (which answers every INVITE with media at `127.0.0.1:40010`), and `wait_registered`. The `FakeReceiver` is `Clone` with `Arc` state (Task 3), so we clone a `probe` before moving one copy into the bridge:

```rust
#[tokio::test]
async fn pull_uses_udp_transport_and_skips_connect() {
    let scfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    let (server, mut server_events) = GbServer::bind(scfg).await.unwrap();
    let server_addr = server.local_addr();
    let fake = FakeReceiver::default();
    let probe = fake.clone();
    let bridge = GbBridge::new(server, "127.0.0.1".into(), Box::new(fake));

    let (client, answerer) = spawn_answering_client(server_addr).await;
    wait_registered(&mut server_events).await;

    bridge.register_mapping("cam1", DEVICE, CHANNEL, gb28181::Transport::Udp);
    assert!(bridge.handle_media_not_found("cam1").await);
    assert!(bridge.is_active("cam1"));
    assert_eq!(probe.opened.lock().unwrap()[0].1, gb28181::Transport::Udp);
    assert!(probe.connected.lock().unwrap().is_empty());

    client.shutdown();
    answerer.abort();
}

#[tokio::test]
async fn pull_active_does_two_phase_connect() {
    let scfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    let (server, mut server_events) = GbServer::bind(scfg).await.unwrap();
    let server_addr = server.local_addr();
    let fake = FakeReceiver::default();
    let probe = fake.clone();
    let bridge = GbBridge::new(server, "127.0.0.1".into(), Box::new(fake));

    let (client, answerer) = spawn_answering_client(server_addr).await;
    wait_registered(&mut server_events).await;

    bridge.register_mapping("cam2", DEVICE, CHANNEL, gb28181::Transport::TcpActive);
    assert!(bridge.handle_media_not_found("cam2").await);
    assert!(bridge.is_active("cam2"));
    assert_eq!(probe.opened.lock().unwrap()[0].1, gb28181::Transport::TcpActive);
    // the answering client answered with media at 127.0.0.1:40010 -> connect there
    assert_eq!(
        probe.connected.lock().unwrap().as_slice(),
        &["127.0.0.1:40010".parse().unwrap()]
    );

    client.shutdown();
    answerer.abort();
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p nvr --bin nvr gb::bridge 2>&1 | tail -20`
Expected: FAIL — pull still uses hardcoded `Udp`; active never connects.

- [ ] **Step 3: Implement the two-phase pull** — replace `start_pull` in `nvr/src/gb/bridge.rs`:

```rust
    async fn start_pull(
        &self,
        stream_id: &str,
        mapping: &crate::gb::stream_map::Mapping,
    ) -> anyhow::Result<()> {
        let transport = mapping.transport;
        let handle = self.receiver.open(stream_id, transport).await?;
        let port = handle.port();
        let (ssrc, ssrc_str) = self.server.next_ssrc(SsrcKind::Live);
        let media_addr = format!("{}:{}", self.media_ip, port).parse()?;
        let spec = MediaSpec {
            ssrc,
            ssrc_str,
            transport,
            media_addr,
            stream_type: StreamType::Play,
            negotiated_remote: None,
        };
        let session = self
            .server
            .invite_play(&mapping.device_id, &mapping.channel_id, spec)
            .await?;
        // TCP-active: now that the device answered with its media address, connect
        // out to it. UDP / TCP-passive: the device pushes to `media_addr`, nothing
        // more to do.
        if transport == Transport::TcpActive {
            let remote = session.spec.negotiated_remote.ok_or_else(|| {
                anyhow::anyhow!("gb28181: tcp-active pull for {stream_id} got no negotiated remote")
            })?;
            handle.connect(remote).await?;
        }
        self.active.lock().unwrap().insert(
            stream_id.to_string(),
            ActiveSession { _receiver: handle, session },
        );
        log::info!(
            "gb28181: pulling {} channel {} -> stream {} ({:?} port {})",
            mapping.device_id,
            mapping.channel_id,
            stream_id,
            transport,
            port
        );
        Ok(())
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nvr --bin nvr gb:: 2>&1 | tail -20`
Expected: PASS — the two new cases + all existing gb tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add nvr/src/gb/bridge.rs nvr/src/gb/bridge_test.rs
git commit -m "feat(gb28181): transport-aware two-phase pull (udp/passive/active)"
```

---

## Task 5: POST /gb/play

**Files:**
- Modify: `nvr/src/gb/api.rs`

- [ ] **Step 1: Write the failing test** — add a `#[cfg(test)]` module at the bottom of `nvr/src/gb/api.rs` (or a colocated `api_test.rs` per repo convention) exercising the pure transport parse:

```rust
#[cfg(test)]
mod play_tests {
    use super::*;

    #[test]
    fn parse_transport_maps_known_values_and_defaults() {
        assert_eq!(parse_transport(None), Some(gb28181::Transport::Udp));
        assert_eq!(parse_transport(Some("udp")), Some(gb28181::Transport::Udp));
        assert_eq!(parse_transport(Some("tcp_passive")), Some(gb28181::Transport::TcpPassive));
        assert_eq!(parse_transport(Some("tcp_active")), Some(gb28181::Transport::TcpActive));
        assert_eq!(parse_transport(Some("bogus")), None);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nvr --bin nvr gb::api 2>&1 | tail -20`
Expected: FAIL — `parse_transport` not defined.

- [ ] **Step 3: Implement** — in `nvr/src/gb/api.rs`:

Add the route in `gb_router`:
```rust
        .route("/play", post(play))
```
Add the request/response types + handler + parser:
```rust
#[derive(Deserialize)]
struct PlayRequest {
    /// The nvr device id (== ZLM stream id).
    device_id: String,
    /// "udp" | "tcp_passive" | "tcp_active"; defaults to "udp" when omitted.
    #[serde(default)]
    transport: Option<String>,
}

#[derive(Serialize)]
struct PlayResponse {
    stream_id: String,
    url: String,
}

/// Parse the transport string; `None` (missing) defaults to Udp; an unknown
/// value yields `None` (rejected by the handler).
fn parse_transport(s: Option<&str>) -> Option<gb28181::Transport> {
    match s {
        None | Some("udp") => Some(gb28181::Transport::Udp),
        Some("tcp_passive") => Some(gb28181::Transport::TcpPassive),
        Some("tcp_active") => Some(gb28181::Transport::TcpActive),
        Some(_) => None,
    }
}

/// Set the transport for a configured gb stream and return its playable URL.
/// `device_id` is the nvr device id (== stream id); the mapping must already
/// exist (registered from the gb device config at startup).
async fn play(Json(req): Json<PlayRequest>) -> ApiJsonResult<PlayResponse> {
    let Some(bridge) = crate::gb::bridge() else {
        return Err(anyhow::anyhow!("GB support is not enabled").into());
    };
    let transport = parse_transport(req.transport.as_deref())
        .ok_or_else(|| anyhow::anyhow!("unknown transport: {:?}", req.transport))?;
    if !bridge.set_transport(&req.device_id, transport) {
        return Err(anyhow::anyhow!("no gb stream mapping for device {}", req.device_id).into());
    }
    Ok(ok_json(PlayResponse {
        stream_id: req.device_id.clone(),
        url: crate::init::device::build_gb_flv_url(&req.device_id),
    }))
}
```
Add a passthrough on `GbBridge` (in `nvr/src/gb/bridge.rs`):
```rust
    /// Update the transport of an existing stream mapping. False if absent.
    pub fn set_transport(&self, stream_id: &str, transport: gb28181::Transport) -> bool {
        self.streams.set_transport(stream_id, transport)
    }
```

- [ ] **Step 4: Run test + build**

Run: `cargo test -p nvr --bin nvr gb::api 2>&1 | tail -20 && cargo check -p nvr 2>&1 | tail -5`
Expected: `parse_transport_maps_known_values_and_defaults` PASS; crate `Finished`.

- [ ] **Step 5: Full gb suite + fmt**

Run: `cargo fmt && cargo test -p nvr --bin nvr gb:: 2>&1 | tail -15`
Expected: all gb tests PASS.

- [ ] **Step 6: Commit**

```bash
git add nvr/src/gb/api.rs nvr/src/gb/bridge.rs
git commit -m "feat(gb28181): POST /gb/play sets per-request transport, returns play url"
```

---

## Final verification (after all tasks)

```bash
export CARGO_TARGET_DIR=/root/workspace/master/lite-nvr/target-gb28181
export LD_LIBRARY_PATH="$PWD/ffmpeg/lib:$PWD/zlm/lib:$LD_LIBRARY_PATH"
export ZLM_DIR="$(ls -d $PWD/target*/debug/build/rszlm-sys-*/out/zlm-install 2>/dev/null | head -1)"
cargo fmt --check
cargo check -p nvr 2>&1 | tail -5           # zero warnings
cargo test -p nvr --bin nvr gb:: 2>&1 | tail -15
cargo test -p nvr --bin nvr zlm::cmd 2>&1 | tail -15
grep -rn "unsafe impl Send" nvr/src/gb/     # expect: no matches
```

Manual/integration (needs a real gb camera; out of scope for automated tests):
`POST /api/gb/play {device_id, transport:"tcp_active"}` then open the returned URL — verify the device is INVITEd and media flows over TCP-active (RtpServer connects out to the answer's media addr).

---

## Notes / deferrals (P4b, not this plan)

- `MediaCache` + `on_media_changed` hook, `rtp_info`-based post-INVITE verify, `GET /gb/streams`, and dashboard status — all P4b. `ZlmControl::rtp_info` is implemented here but has no consumer yet in P4a.
- `UnRegist`-driven teardown is a deliberate non-goal (spec §3.4).
