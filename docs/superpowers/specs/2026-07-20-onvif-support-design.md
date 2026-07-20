# ONVIF support: discovery + ingestion + PTZ

Date: 2026-07-20
Status: approved (full scope: WS-Discovery + RTSP ingestion + PTZ; base
library lumeohq/onvif-rs git-pinned; leaf crate `crates/onvif` + `nvr/src/onvif`
integration; RTSP resolved on connect via a supervisor, config stored on the
device)

## Problem

lite-nvr ingests RTSP, GB28181, Xiaomi, and platform live streams, but has no
support for **ONVIF** — the standard IP-camera protocol for LAN discovery,
media-profile/stream-URI negotiation, and PTZ. Unlike GB28181 (where cameras
*register into* our SIP platform), ONVIF makes the NVR the **client**: we query
the camera over SOAP/HTTP to discover it, fetch its RTSP URI, and drive PTZ.
Once we hold the RTSP URI, media flows through the **existing RTSP → ZLM device
pipeline** — so ONVIF is a *resolution + control* layer, not a new media path,
much closer to the `stream` (yt-dlp) type than to the GB28181 subsystem.

## Base library

`lumeohq/onvif-rs` — the most complete native-Rust ONVIF **client** (types
generated from official WSDL/XSD; WS-Discovery, WS-Security UsernameToken auth,
device/media/PTZ services, async/tokio). Pure Rust — no ffmpeg `links=`
conflict. No crates.io release, so it is pinned as a **git dependency at a
specific commit**; its WSDL/XSD-generated schema adds some compile time
(isolated inside the leaf crate). Client-only (no ONVIF server), which is
exactly the NVR's role.

## Architecture

Two units, mirroring the `crates/gb28181` + `nvr/src/gb` split:

1. **`crates/onvif`** — a thin, testable client wrapper. No nvr/ffmpeg/zlm types.
2. **`nvr/src/onvif/`** — REST surface, device registry, and the ingestion
   supervisor that wires the crate into the existing device/ZLM pipeline.

Plus frontend additions in `nvr-dashboard`.

### `crates/onvif` public API

```rust
/// Connection config for one ONVIF camera. Serde — this is exactly what an
/// `input_type == "onvif"` device stores in `input_value` (password included).
#[derive(Clone, Serialize, Deserialize)]
pub struct OnvifConfig {
    pub host: String,
    pub port: u16,                    // ONVIF service port (often 80 or 8000)
    pub username: String,
    pub password: String,
    pub profile_token: Option<String>, // chosen media profile; None = first
}

/// A camera found on the LAN via WS-Discovery.
pub struct Discovered {
    pub endpoints: Vec<String>,       // xaddr service URLs
    pub name: Option<String>,
    pub hardware: Option<String>,
    pub addr: Option<String>,         // host:port parsed from the first xaddr
}

pub struct DeviceInfo { pub manufacturer: String, pub model: String,
                        pub firmware: String, pub serial: String }
pub struct Profile { pub token: String, pub name: String,
                     pub width: u32, pub height: u32,
                     pub video_codec: String, pub fps: f32 }
pub struct Preset { pub token: String, pub name: String }

/// Continuous-move velocity, each axis in -1.0..=1.0 (0 = no motion).
pub struct PtzVelocity { pub pan: f32, pub tilt: f32, pub zoom: f32 }

/// WS-Discovery probe on the LAN; returns after `timeout`.
pub async fn discover(timeout: Duration) -> Result<Vec<Discovered>, OnvifError>;

pub struct OnvifCamera { /* device + media + ptz service clients */ }

impl OnvifCamera {
    pub async fn connect(cfg: &OnvifConfig) -> Result<OnvifCamera, OnvifError>;
    pub async fn device_info(&self) -> Result<DeviceInfo, OnvifError>;
    pub async fn profiles(&self) -> Result<Vec<Profile>, OnvifError>;
    /// RTSP URI for `profile` (or the config's/first profile). RTP-over-RTSP.
    pub async fn stream_uri(&self, profile: Option<&str>) -> Result<String, OnvifError>;
    pub async fn ptz_move(&self, v: PtzVelocity) -> Result<(), OnvifError>;
    pub async fn ptz_stop(&self) -> Result<(), OnvifError>;
    pub async fn presets(&self) -> Result<Vec<Preset>, OnvifError>;
    pub async fn goto_preset(&self, token: &str) -> Result<(), OnvifError>;
}

pub enum OnvifError {
    Connect(String),      // host unreachable / bad port
    Auth,                 // WS-Security rejected (bad user/pass)
    NoPtzService,         // camera has no PTZ service
    NoProfile(String),    // requested profile token absent
    Protocol(String),     // SOAP fault / schema / transport error
}
```

Notes:
- `stream_uri` requests the RTSP-over-TCP transport; the returned URI carries no
  credentials, so the ingestion layer injects `username:password@` into the URI
  before handing it to ffmpeg (many cameras require RTSP digest/basic auth with
  the same ONVIF credentials).
- `connect` resolves the media/PTZ service endpoints from `GetCapabilities` /
  `GetServices` and constructs the per-service clients once.

### `nvr/src/onvif/` integration

- **`mod.rs`** — module wiring + an in-memory registry
  `RwLock<HashMap<String /*device_id*/, OnvifConfig>>`, populated when an
  `onvif` device is added or restored at startup. PTZ and stream re-resolution
  read the config from here (mirrors the gb bridge mapping registry) instead of
  re-reading the DB each call.
- **`api.rs`** — `/api/onvif` router (sits under the existing `/api` session-auth
  middleware), paralleling `/api/gb`:
  | Method | Path | Body / Params | Returns |
  |---|---|---|---|
  | POST | `/api/onvif/discover` | `{ timeout_ms?: u64 }` | `[Discovered]` |
  | POST | `/api/onvif/probe` | `{ host, port, username, password }` | `{ device_info, profiles }` |
  | POST | `/api/onvif/ptz` | `{ device_id, direction, speed?, preset_token? }` | `()` |
  | GET | `/api/onvif/presets/{device_id}` | — | `[Preset]` |

  `direction` uses the **same verb contract as `/api/gb/ptz`**
  (`up`/`down`/`left`/`right`/`zoom_in`/`zoom_out`/`stop`/`preset_call`), mapped
  to `PtzVelocity` continuous-move (verbs → ±axis at `speed/255`, default speed
  128), `ptz_stop`, or `goto_preset`. Unlike GB28181 (numeric preset slot in the
  speed byte), ONVIF presets are **string tokens**, so `direction == "preset_call"`
  requires `preset_token` in the body (the token comes from
  `GET /api/onvif/presets/{device_id}`); it is ignored for the other verbs.
  `probe` connects ad-hoc (not from the registry) so the add-device form can
  validate credentials and list profiles before saving.
- **Ingestion (`init/device.rs`)** — new branch `input_type == "onvif"`: parse
  `OnvifConfig` from `input_value`, register it in the registry, create the ZLM
  `device/<id>` media, and spawn a **resolve-on-connect supervisor that is a
  near-copy of `livestream::spawn_stream_device`** (the yt-dlp path):

  ```
  loop {
      cancel? break
      let uri = OnvifCamera::connect(cfg).stream_uri(cfg.profile_token)   // resolve
                  → inject credentials into the URI
      run_session(rtsp uri → ZLM media)                                   // pull
      cancel? break
      backoff (2s → 60s, reset after a healthy ≥30s session); re-resolve
  }
  ```

  So a camera IP/credential change or reboot self-heals on the next reconnect,
  and live FLV/HLS + recording behave exactly like any RTSP device. The supervisor
  is registered in the manager as a `Task` entry, stopped via its `CancellationToken`
  (same lifecycle as the `stream` type).
- **`api.rs` mount** — add `.nest("/onvif", crate::onvif::api::onvif_router())`
  in `nvr/src/api.rs` alongside the other routers (inside the auth-guarded `/api`).

### Frontend (`nvr-dashboard`)

- **`src/api/onvif.ts`** — `discover`, `probe`, `ptz`, `getPresets` clients.
- **DeviceListView** — add `{ label: "ONVIF 摄像头", value: "onvif" }` to
  `inputTypeOptions`. When `onvif` is selected, show host/port/username/password
  fields plus:
  - a **「探测」** button → `probe` → renders `device_info` and a **profile
    picker** (`Select` over the returned profiles);
  - an optional **「扫描局域网」** button → `discover` → lists found cameras;
    picking one prefills host/port.
  On save, `input_value` = `OnvifConfig` JSON (host/port/user/pass/profile_token).
- **PTZ controls** — reuse the existing gb PTZ control UI/pattern (same direction
  verbs) on ONVIF device rows, routed to `/api/onvif/ptz`; a presets dropdown
  calls `/api/onvif/presets/{id}` + `preset_call`.

## Data flow

Add device (onvif) → 「探测」 validates creds + lists profiles → pick profile →
save `OnvifConfig` JSON → supervisor resolves RTSP URI (credentials injected) →
`run_session` pulls into ZLM `device/<id>` → existing live/record pipeline. PTZ:
dashboard → `/api/onvif/ptz` → registry lookup → `OnvifCamera` ContinuousMove/Stop.

## Error handling

- **`probe`** returns the mapped `OnvifError` (Connect / Auth / Protocol) as a
  normal API error so the form shows an actionable message.
- **Supervisor** resolution failures log + back off + retry (transient network,
  camera reboot); they do NOT tear down the device.
- **PTZ** on a camera without a PTZ service → `NoPtzService` → clear API error.
- **`ptz` for an unknown `device_id`** (not in the registry) → 4xx-style API error.

## Testing

- **`crates/onvif` unit tests (no network):**
  - WS-Discovery `ProbeMatches` XML → `Discovered` parsing.
  - `GetProfiles` response → `Profile` mapping (token/resolution/codec/fps).
  - PTZ verb + speed → `PtzVelocity` mapping (and `stop`/`preset_call` dispatch),
    with velocity clamped to -1.0..=1.0.
  - `OnvifConfig` serde round-trip.
- **`crates/onvif` ignored integration test:** `#[ignore]`, reads
  `ONVIF_TEST_HOST`/`ONVIF_TEST_USER`/`ONVIF_TEST_PASS`; skips if unset. Against
  a real camera or an ONVIF simulator (e.g. ONVIF Device Manager / happytimesoft
  simulator): `connect` → `device_info` → `profiles` → `stream_uri` → a PTZ
  move+stop.
- **nvr:** handler unit test for the `direction` → PTZ mapping and the
  registry-miss error path (with a stubbed camera); manual end-to-end against a
  real/sim camera confirming live view + recording + PTZ.

## Out of scope (deliberate, v1)

- ONVIF **event subscription** (motion/analytics PullPoint / base notification).
- **Imaging** settings (brightness/focus/IR-cut), Profile G on-camera recording,
  audio backchannel.
- Multiple simultaneous profiles per device (one chosen profile per device).
- ONVIF **server/device-side** emulation (we are client-only).
- Wiring ONVIF discovery results into any auto-provisioning; discovery only
  prefills the manual add form.
