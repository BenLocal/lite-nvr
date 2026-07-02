# GB28181 ZLM Control Layer + Full Transport — Design

**Status:** Approved (brainstorm), pending spec review
**Date:** 2026-07-02
**Depends on:** P1-3 nvr bridge (on-demand pull), rszlm ≥ `0e1af4d` (v0.2.0 — `RtpServerTcpMode`, `RtpServer::connect`, `rtp_get_info`, `on_media_changed`)
**Supersedes the P1-3 "UDP-only, TCP deferred" note** in the gb bridge.

---

## 1. Goal

Turn the P1-3 UDP-only on-demand bridge into a full-transport (UDP / TCP-passive /
TCP-active) puller, and add a small ZLM control/observability layer:

- A **dedicated ZLM worker thread** (`ZlmControl`) that owns all `RtpServer`s and
  runs ZLM's synchronous control FFI (`RtpServer` create/connect/close,
  `rtp_get_info`) off the async runtime, behind an async command channel (`ZlmCmd`).
- A **`POST /gb/play`** endpoint that selects the transport per request (stored on
  the stream mapping) and returns the playable URL; the pull stays lazy
  (`on_media_not_found`-triggered).
- An **`on_media_changed` cache** + **`rtp_get_info`** query serving both internal
  pull-robustness and a **`GET /gb/streams`** status API for the dashboard.

## 2. Background (current state, verified)

- `stream_id` **is the nvr device id**. Mappings are registered at startup
  (`nvr/src/init/device.rs:79`) for `input_type=gb28181` devices whose
  `input_value` is `{ "device_id": <gb>, "channel_id": <gb> }`, and are removed on
  device update/delete (`nvr/src/handler/device.rs`).
- gb streams are published by ZLM's `RtpServer` under the **`rtp` app**; the
  playable URL is `build_gb_flv_url(device_id)` →
  `http://127.0.0.1:8553/rtp/{device_id}.live.flv` (`nvr/src/init/device.rs:139`).
- `nvr/src/gb/bridge.rs::start_pull` **hardcodes `Transport::Udp`**; the receiver
  rejects TCP (`nvr/src/gb/receiver.rs`). Everything else for TCP is already in
  place: the crate's `sdp.rs` emits `a=setup:passive`/`a=setup:active` offers and
  parses the answer into `spec.negotiated_remote`, and `server.rs::invite_play`
  already stores `negotiated_remote` "needed for TcpActive connect".
- Today the `RtpServer` is held by the bridge with a hand-written
  `unsafe impl Send for ZlmReceiverHandle`. This design removes that.

## 3. Settled decisions (from brainstorm)

1. **Transport is per-request**, delivered via an explicit **`POST /gb/play`**
   endpoint that sets the transport on the mapping; the pull stays **lazy**
   (viewer opens URL → `on_media_not_found` → bridge reads `mapping.transport`).
   (Rejected: per-device config, global default, stream_id encoding.)
2. **`ZlmCmd` executes on a dedicated ZLM worker thread** that owns all
   `RtpServer`s keyed by `stream_id`. The `RtpServer` never leaves that thread →
   **no `unsafe impl Send`**; the receiver handle carries only `{stream_id, sender}`.
   (Rejected: `spawn_blocking`-per-op, which would keep the unsafe-Send wrapper.)
3. **The media-state layer serves both** internal pull-robustness and the external
   `GET /gb/streams` API.
4. **`UnRegist` does NOT drive teardown.** The `on_media_changed` cache is used
   only to (a) confirm a pull actually registered a stream and (b) report status.
   Teardown stays on `on_media_no_reader`, to avoid fighting our own close / re-pull.

## 4. Architecture

```
POST /gb/play {device_id, transport}                 GET /gb/streams
        │ set mapping.transport, return url                 │ snapshot
        ▼                                                    ▼
   StreamMap(+transport) ◄─────────── GbBridge ───────────► MediaCache ◄─ on_media_changed hook
                                        │  (start_pull, two-phase)          (zlm/server.rs)
                                        ▼
                                   ZlmControl  ── ZlmCmd ──►  ZLM worker thread
                                   (async facade)             owns HashMap<stream_id, RtpServer>
                                                              OpenRtp/ConnectRtp/CloseRtp/GetRtpInfo
```

### 4.1 `nvr/src/zlm/control.rs` (new) — ZLM control worker

```rust
pub enum ZlmCmd {
    OpenRtp    { stream_id: String, mode: RtpServerTcpMode, reply: oneshot::Sender<anyhow::Result<u16>> },
    ConnectRtp { stream_id: String, remote: SocketAddr,     reply: oneshot::Sender<anyhow::Result<()>> },
    CloseRtp   { stream_id: String },
    GetRtpInfo { app: String, stream: String,               reply: oneshot::Sender<Option<RtpInfo>> },
}

/// Cheap-clone async facade over the worker's command channel.
#[derive(Clone)]
pub struct ZlmControl { tx: std::sync::mpsc::Sender<ZlmCmd> }

impl ZlmControl {
    pub fn spawn() -> Self;                                              // starts the worker thread
    pub async fn open_rtp(&self, stream_id: &str, mode: RtpServerTcpMode) -> anyhow::Result<u16>;
    pub async fn connect_rtp(&self, stream_id: &str, remote: SocketAddr) -> anyhow::Result<()>;
    pub fn close_rtp(&self, stream_id: &str);                            // fire-and-forget
    pub async fn rtp_info(&self, app: &str, stream: &str) -> Option<RtpInfo>;
}
```

Worker loop: `let mut servers: HashMap<String, RtpServer> = ...; while let Ok(cmd) = rx.recv() { handle(cmd, &mut servers) }`.

- **OpenRtp** → `RtpServer::new(0, mode, &stream_id)`, insert into `servers`,
  `reply.send(Ok(server.bind_port()))`.
- **ConnectRtp** → look up server; `server.connect(&remote.ip().to_string(), remote.port(), move |code, msg, _| { let _ = reply.send(if code == 0 { Ok(()) } else { Err(anyhow!("rtp connect failed: {code} {msg}")) }); })`.
  **The `reply` is moved into the callback** — the connect result arrives
  asynchronously on a ZLM thread, not from the handler's return.
- **CloseRtp** → `servers.remove(&stream_id)` (drops `RtpServer`, releases port).
- **GetRtpInfo** → `reply.send(rszlm::server::rtp_get_info(&app, &stream))`.
  Self-contained (thread-local slot set+taken within the call); always runs on the
  worker thread so the slot is consistent.

Each `ZlmControl` async method builds a `tokio::sync::oneshot`, sends the `ZlmCmd`,
and awaits the reply (`RecvError` → surfaced as an error, e.g. worker gone).

### 4.2 `nvr/src/zlm/media_cache.rs` (new) — on_media_changed cache

```rust
#[derive(Clone, Default)]
pub struct MediaCache { inner: Arc<Mutex<HashMap<(String, String), MediaEntry>>> } // key = (app, stream)

pub struct MediaEntry { /* presence = live; room for schema/regist time if needed */ }

impl MediaCache {
    pub fn on_regist(&self, app: &str, stream: &str);     // Regist  -> insert
    pub fn on_unregist(&self, app: &str, stream: &str);   // UnRegist-> remove
    pub fn is_live(&self, app: &str, stream: &str) -> bool;
    pub fn live_streams(&self) -> Vec<(String, String)>;
}
```

Registered via ZLM's global `on_media_changed` hook in `nvr/src/zlm/server.rs`
alongside the existing `on_media_not_found`/`on_media_no_reader` hooks:
`Regist(src) => cache.on_regist(&src.app(), &src.stream())`,
`UnRegist(src) => cache.on_unregist(...)`.

### 4.3 `nvr/src/gb/receiver.rs` (rework) — MediaReceiver over ZlmControl

- `MediaReceiver::open(stream_id, transport) -> anyhow::Result<Box<dyn ReceiverHandle>>`
  maps `Udp→Disabled`, `TcpPassive→Passive`, `TcpActive→Active`, calls
  `control.open_rtp(stream_id, mode).await`, and returns a handle whose `Drop`
  calls `control.close_rtp(stream_id)`. The handle exposes `port()`.
- The real handle now holds `{ stream_id: String, port: u16, control: ZlmControl }`
  — **all `Send`; the `unsafe impl Send` is deleted.**
- The `#[cfg(test)]` fake receiver stays, extended to record the `transport` it was
  opened with and any `connect` calls (see §7).

> Note: `open` is `async` under this design (it awaits `open_rtp`). The trait
> becomes `#[async_trait]` (or `open` returns a future) — the plan picks the
> mechanism to match the codebase; the fake mirrors it.

### 4.4 `nvr/src/gb/bridge.rs` (rework `start_pull`) — transport-aware two-phase pull

```
mode   = map(mapping.transport)
handle = receiver.open(stream_id, mapping.transport).await?   // worker creates RtpServer, returns port
port   = handle.port()
spec   = MediaSpec { transport: mapping.transport, media_addr: media_ip:port, ssrc, stream_type: Play, .. }
session= server.invite_play(dev, chan, spec).await?           // fills negotiated_remote
if mapping.transport == TcpActive {
    control.connect_rtp(stream_id, session.negotiated_remote.unwrap()).await?  // else: BYE + drop handle
}
active.insert(stream_id, ActiveSession { handle, session })
```

- The bridge holds a `ZlmControl` handle (for the active-mode `connect_rtp`; the
  receiver already carries its own clone for open/close).
- **Optional light verify (internal robustness):** after inserting, the bridge may
  check `control.rtp_info("rtp", stream_id)` after a short delay; if `!exist`, tear
  the session down so a ZLM re-fire retries. Bounded, best-effort, non-blocking.
- Teardown unchanged (`no_reader` → BYE + handle Drop → CloseRtp).

### 4.5 `nvr/src/gb/stream_map.rs` (change) — Mapping gains transport

`Mapping { device_id, channel_id, transport: Transport }`;
`register(stream_id, device_id, channel_id, transport)`. Startup registration
(`init/device.rs`) passes `Transport::Udp` (the default); `POST /gb/play` overwrites
it. Add `set_transport(stream_id, transport) -> bool` (or re-`register`) for the play
endpoint, and `list() -> Vec<(String, Mapping)>` (stream_id + mapping) for
`GET /gb/streams`.

### 4.6 `nvr/src/gb/api.rs` (change) — play + streams endpoints

- **`POST /gb/play { device_id, transport? }`** — `device_id` is the **nvr device
  id (= stream_id)**. Sets `mapping.transport` (default `udp` when omitted) on the
  already-registered mapping; returns `{ stream_id, url }` where
  `url = build_gb_flv_url(device_id)`. Errors if no mapping / GB disabled.
  `transport ∈ {"udp","tcp_passive","tcp_active"}`.
- **`GET /gb/streams`** — for each mapping, report
  `{ stream_id, device_id, channel_id, transport, live, rtp }` where `live` comes
  from `MediaCache.is_live("rtp", stream_id)` and `rtp` (optional) from
  `control.rtp_info("rtp", stream_id)` (`peer_ip, peer_port, local_port, ssrc/identifier`).
- Router (`gb_router`) adds `.route("/play", post(play))` and
  `.route("/streams", get(streams))`. **HTTP verbs: GET/POST only** (project rule).

### 4.7 `nvr/src/zlm/server.rs` + startup wiring (change)

- Register the `on_media_changed` hook → `MediaCache`.
- Construct `ZlmControl` (spawns the worker) and `MediaCache` at startup; inject
  both into `GbBridge::new(server, media_ip, receiver, control, media_cache)` (the
  receiver is built from the same `ZlmControl`).

## 5. Data flow

```
POST /gb/play{device_id, transport}
  -> streams.set_transport(device_id, transport)      -> {stream_id: device_id, url}
viewer opens url
  -> ZLM on_media_not_found(app="rtp", stream=device_id)
  -> bridge.handle_media_not_found(stream_id):
       mapping = streams.get(stream_id)                // {gb device_id, channel_id, transport}
       mode = map(mapping.transport)
       port = control.open_rtp(stream_id, mode)        // worker: RtpServer::new(0, mode, stream_id)
       session = server.invite_play(gb_dev, gb_chan, spec{transport, media_ip:port})
       if active: control.connect_rtp(stream_id, session.negotiated_remote)
       active.insert(...)                              // ZLM receives PS/RTP -> publishes
  -> on_media_changed Regist("rtp", stream_id)         -> cache live=true
viewer leaves
  -> on_media_no_reader(stream_id) -> teardown: session BYE + handle Drop -> control.close_rtp
  -> on_media_changed UnRegist("rtp", stream_id)       -> cache live=false
```

## 6. Error handling

| Failure | Behaviour |
|---|---|
| `open_rtp` bind fails | error; nothing inserted; handle not created; ZLM re-fire retries |
| INVITE rejected / device offline | `DeviceOffline`/`Negotiation` error; handle Drop releases port |
| active `connect_rtp` fails (device unreachable) | tear down the just-opened handle + BYE; error logged; re-fire retries |
| `connect_rtp` reply never arrives (no connect callback) | `oneshot` recv errors after the pull's own timeout path; treated as connect failure |
| worker thread dead | async methods get `RecvError` → surfaced as error (should not happen; worker lives for process) |
| `rtp_info` `exist:false` | API reports not-yet-receiving; optional internal verify may retry the pull |
| `POST /gb/play` unknown device / GB disabled | 200-envelope error (uniform with existing gb API) |

## 7. Testing

- **`media_cache.rs`** — pure unit tests: regist/unregist/is_live/live_streams.
- **`control.rs`** — unit-test the command *routing* / facade where possible
  without ZLM (e.g. reply plumbing); the `RtpServer`/`rtp_get_info` FFI itself is
  covered by rszlm. No new native-lib requirement for the pure parts.
- **`bridge.rs`** — extend the fake `MediaReceiver` to record the `transport` per
  `open` and to record `connect(remote)`; assert `start_pull` (a) opens with
  `mapping.transport`, (b) for `TcpActive` performs the two-phase open→invite→connect
  with the answer's `negotiated_remote`, (c) UDP/passive skip connect. Reuse the
  existing fake `GbServer`/loopback harness.
- **`api.rs`** — `POST /gb/play` sets transport + returns the `rtp`-app url;
  `GET /gb/streams` shape (live flag + optional rtp block).
- **SDP/transport** — already covered in `crates/gb28181/src/sdp.rs` tests.

## 8. Decomposition (for writing-plans)

One spec, two implementation plans (P4b builds on P4a's `ZlmControl`):

- **P4a — transport + control core:** `ZlmControl` worker + `ZlmCmd`; receiver
  reworked over it (delete `unsafe impl Send`); `Mapping.transport`; transport-aware
  two-phase `start_pull` (UDP / passive / active); `POST /gb/play`. Delivers the
  actual full-transport pull.
- **P4b — media-state layer:** `MediaCache` + `on_media_changed` hook; `rtp_info`
  async + optional pull verify; `GET /gb/streams` + dashboard status column.

## 9. Non-goals / deferrals

- `UnRegist`-driven teardown (see §3.4).
- Multiplexed single-port RTP (`RtpServerBuilder` multiplex) — one `RtpServer` per
  stream, as today.
- `transcode` feature (default-off in rszlm; not linked).
- Per-device default transport in `input_value` — transport is per-play-request
  only; the mapping default is `udp`.
- PTZ/position-query and other DeviceControl subtypes (P2 scope).

## 10. Open point for spec review

`POST /gb/play` keys off the **nvr device id (= stream_id)** and updates the
transport on the mapping registered from device config, rather than taking the raw
GB `device_id`+`channel_id` and minting a new mapping. This matches the existing
config-driven registration and the dashboard's device identity, and avoids the
`device_id` (nvr vs GB) ambiguity. Confirm this contract during spec review.
