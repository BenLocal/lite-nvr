# ONVIF Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add ONVIF camera support to lite-nvr — LAN discovery, RTSP-URI ingestion (via the existing RTSP→ZLM pipeline), and PTZ — as a leaf crate `crates/nvr-onvif` plus `nvr/src/onvif` integration and dashboard UI.

**Architecture:** `crates/nvr-onvif` is a thin async client over the upstream `onvif` crate (lumeohq/onvif-rs, git-pinned): discovery, connect, profiles, stream-URI, PTZ. `nvr/src/onvif` adds a `/api/onvif` REST surface, an in-memory `device_id → OnvifConfig` registry, and a resolve-on-connect ingestion supervisor cloned from the yt-dlp `stream` type so media reuses the existing RTSP device pipeline. The dashboard gets an ONVIF device form (probe + discover) and PTZ controls.

**Tech Stack:** Rust edition 2024, upstream `onvif` crate (git), `tokio`/`tokio-util`, `serde`, `anyhow`, `futures`, `url`; Axum for REST; Vue 3 + PrimeVue for the dashboard.

## Global Constraints

- Rust **edition 2024**; `snake_case`; run `cargo fmt` before every commit.
- The leaf crate is named **`nvr-onvif`** (NOT `onvif` — it depends on the upstream `onvif` crate; a package can't share its own dependency's name). Directory `crates/nvr-onvif`, lib name `nvr_onvif`.
- Do **not** modify `crates/ffmpeg-bus`. ONVIF adds **no new media path** — ingestion resolves an RTSP URI and reuses the existing RTSP→ZLM device pipeline.
- Tests colocated as `<module>_test.rs`, imported via `#[cfg(test)] #[path = "<module>_test.rs"] mod <module>_test;` (repo convention in `CLAUDE.md`).
- `nvr-onvif` is **pure Rust (no ffmpeg linkage)**, so `cargo test -p nvr-onvif` needs no `LD_LIBRARY_PATH`. Any command that builds/tests **`nvr`** must be run from the repo root prefixed with `LD_LIBRARY_PATH=$PWD/ffmpeg/lib` (e.g. `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo check -p nvr`).
- `onvif` (upstream) has no crates.io release → declared as a **git dependency**; `Cargo.lock` pins the exact commit. Record the resolved rev in the Task 1 report.
- REST convention: **GET/POST only**. `/api/onvif/discover|probe|ptz` are POST; `/api/onvif/presets/{id}` is GET. All under the existing `/api` session-auth middleware (no extra auth code needed).
- **PTZ verb contract matches `/api/gb/ptz`**: `direction ∈ {up,down,left,right,zoom_in,zoom_out,stop,preset_call}`, optional `speed` (0..=255, default 128) mapped to velocity `speed/255`, optional `preset_token` (required for `preset_call`, ignored otherwise).
- **onvif-rs API caveat:** the exact struct/field/function names of the upstream `onvif`/`schema` crates are version-specific. In Tasks 4/5/6/9 the onvif-rs call code is a **starting shape** — adapt it (mirroring the upstream `onvif/examples/camera.rs`) until it compiles, WITHOUT changing the public method signatures declared in each task's Interfaces block. The compile is the gate for those calls; runtime is validated by the ignored live test / manual run.

---

### Task 1: Scaffold `nvr-onvif` crate + `OnvifConfig` (dependency go/no-go)

**Files:**
- Create: `crates/nvr-onvif/Cargo.toml`
- Create: `crates/nvr-onvif/src/lib.rs`
- Create: `crates/nvr-onvif/src/config.rs`
- Create: `crates/nvr-onvif/src/config_test.rs`
- Modify: `Cargo.toml` (workspace `members`)

**Interfaces:**
- Produces: `OnvifConfig { host: String, port: u16, username: String, password: String, profile_token: Option<String> }` (`#[derive(Clone, Debug, Serialize, Deserialize)]`) with `OnvifConfig::service_url(&self) -> String` returning `http://{host}:{port}/onvif/device_service`.

> This task is the **go/no-go on the upstream `onvif` dependency**: Step 2's build compiles onvif-rs and its generated schema. If it fails to resolve/compile in this workspace (version conflicts, schema-gen failure), STOP and report BLOCKED with the error — the rest of the plan depends on it.

- [ ] **Step 1: Create the crate manifest**

Create `crates/nvr-onvif/Cargo.toml`:

```toml
[package]
name = "nvr-onvif"
version = "0.1.0"
edition = "2024"
publish = false
description = "ONVIF client wrapper (discovery, profiles, stream-URI, PTZ) over lumeohq/onvif-rs."

[dependencies]
onvif = { git = "https://github.com/lumeohq/onvif-rs" }
schema = { git = "https://github.com/lumeohq/onvif-rs" }
tokio = { workspace = true }
futures = { workspace = true }
anyhow = { workspace = true }
serde = { workspace = true }
url = "2"

[dev-dependencies]
serde_json = { workspace = true }
tokio = { workspace = true }
```

> Note: the upstream repo publishes both `onvif` (client/soap/discovery) and `schema` (generated SOAP types) as workspace members; both are needed. If `cargo` complains it can't pick a package from the git repo, disambiguate with `package = "onvif"` / `package = "schema"` and the same `git` URL.

- [ ] **Step 2: Register in the workspace and verify the dependency builds**

In root `Cargo.toml`, add to `members` (after `"crates/nvr-recorder",`):

```toml
    "crates/nvr-recorder",
    "crates/nvr-onvif",
```

Run: `cargo build -p nvr-onvif`
Expected: onvif-rs + schema compile (may take a while — schema is codegen-heavy) and the empty crate builds. **If this fails, STOP and report BLOCKED with the exact error.**

- [ ] **Step 3: Write `config.rs`**

Create `crates/nvr-onvif/src/config.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Connection config for one ONVIF camera. This is exactly what an
/// `input_type == "onvif"` device stores in its `input_value` JSON.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnvifConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    /// Chosen media profile token; `None` = use the first profile.
    #[serde(default)]
    pub profile_token: Option<String>,
}

impl OnvifConfig {
    /// The ONVIF device-management service URL (the well-known entry point).
    pub fn service_url(&self) -> String {
        format!("http://{}:{}/onvif/device_service", self.host, self.port)
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
```

- [ ] **Step 4: Write `lib.rs`**

Create `crates/nvr-onvif/src/lib.rs`:

```rust
//! ONVIF client wrapper: discovery, profiles, stream-URI, PTZ.

pub mod config;

pub use config::OnvifConfig;
```

- [ ] **Step 5: Write the failing test**

Create `crates/nvr-onvif/src/config_test.rs`:

```rust
use super::*;

#[test]
fn service_url_uses_host_port() {
    let c = OnvifConfig {
        host: "192.168.1.50".into(),
        port: 8000,
        username: "admin".into(),
        password: "x".into(),
        profile_token: None,
    };
    assert_eq!(
        c.service_url(),
        "http://192.168.1.50:8000/onvif/device_service"
    );
}

#[test]
fn config_serde_round_trip() {
    let json = r#"{"host":"h","port":80,"username":"u","password":"p","profile_token":"Profile_1"}"#;
    let c: OnvifConfig = serde_json::from_str(json).unwrap();
    assert_eq!(c.port, 80);
    assert_eq!(c.profile_token.as_deref(), Some("Profile_1"));
    // profile_token defaults to None when absent
    let c2: OnvifConfig =
        serde_json::from_str(r#"{"host":"h","port":80,"username":"u","password":"p"}"#).unwrap();
    assert_eq!(c2.profile_token, None);
}
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p nvr-onvif`
Expected: PASS (2 tests).

- [ ] **Step 7: Format & commit**

```bash
cargo fmt -p nvr-onvif
git add crates/nvr-onvif Cargo.toml Cargo.lock
git commit -m "feat(nvr-onvif): scaffold crate and OnvifConfig"
```

---

### Task 2: Result types + PTZ velocity (`types.rs`)

**Files:**
- Create: `crates/nvr-onvif/src/types.rs`
- Create: `crates/nvr-onvif/src/types_test.rs`
- Modify: `crates/nvr-onvif/src/lib.rs`

**Interfaces:**
- Produces: `Discovered { endpoints: Vec<String>, name: Option<String>, hardware: Option<String>, addr: Option<String> }`; `DeviceInfo { manufacturer, model, firmware, serial: String }`; `Profile { token: String, name: String, width: u32, height: u32, video_codec: String, fps: f32 }`; `Preset { token: String, name: String }` — all `#[derive(Clone, Debug, Serialize)]`. `PtzVelocity { pan: f32, tilt: f32, zoom: f32 }` with `PtzVelocity::new(pan, tilt, zoom) -> Self` clamping each axis to `-1.0..=1.0`. `OnvifError` enum: `Connect(String)`, `Auth`, `NoPtzService`, `NoProfile(String)`, `Protocol(String)` — `#[derive(Debug)]` + `impl std::fmt::Display` + `impl std::error::Error`.

- [ ] **Step 1: Write `types.rs`**

Create `crates/nvr-onvif/src/types.rs`:

```rust
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Discovered {
    pub endpoints: Vec<String>,
    pub name: Option<String>,
    pub hardware: Option<String>,
    pub addr: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeviceInfo {
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    pub serial: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Profile {
    pub token: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub video_codec: String,
    pub fps: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Preset {
    pub token: String,
    pub name: String,
}

/// Continuous-move velocity; each axis clamped to -1.0..=1.0 (0 = no motion).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PtzVelocity {
    pub pan: f32,
    pub tilt: f32,
    pub zoom: f32,
}

impl PtzVelocity {
    pub fn new(pan: f32, tilt: f32, zoom: f32) -> Self {
        let c = |v: f32| v.clamp(-1.0, 1.0);
        Self { pan: c(pan), tilt: c(tilt), zoom: c(zoom) }
    }
}

#[derive(Debug)]
pub enum OnvifError {
    Connect(String),
    Auth,
    NoPtzService,
    NoProfile(String),
    Protocol(String),
}

impl std::fmt::Display for OnvifError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OnvifError::Connect(s) => write!(f, "connect failed: {s}"),
            OnvifError::Auth => write!(f, "authentication rejected"),
            OnvifError::NoPtzService => write!(f, "camera has no PTZ service"),
            OnvifError::NoProfile(t) => write!(f, "profile not found: {t}"),
            OnvifError::Protocol(s) => write!(f, "onvif protocol error: {s}"),
        }
    }
}

impl std::error::Error for OnvifError {}

#[cfg(test)]
#[path = "types_test.rs"]
mod types_test;
```

- [ ] **Step 2: Export from `lib.rs`**

Add to `crates/nvr-onvif/src/lib.rs`:

```rust
pub mod types;

pub use types::{Discovered, DeviceInfo, OnvifError, Preset, Profile, PtzVelocity};
```

- [ ] **Step 3: Write the failing tests**

Create `crates/nvr-onvif/src/types_test.rs`:

```rust
use super::*;

#[test]
fn velocity_clamps_each_axis() {
    let v = PtzVelocity::new(2.0, -3.0, 0.5);
    assert_eq!(v, PtzVelocity { pan: 1.0, tilt: -1.0, zoom: 0.5 });
}

#[test]
fn error_display_is_human_readable() {
    assert_eq!(format!("{}", OnvifError::Auth), "authentication rejected");
    assert_eq!(
        format!("{}", OnvifError::NoProfile("P1".into())),
        "profile not found: P1"
    );
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p nvr-onvif`
Expected: PASS (4 tests total).

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p nvr-onvif
git add crates/nvr-onvif/src
git commit -m "feat(nvr-onvif): result types, PtzVelocity clamp, OnvifError"
```

---

### Task 3: RTSP credential injection (`uri.rs`)

**Files:**
- Create: `crates/nvr-onvif/src/uri.rs`
- Create: `crates/nvr-onvif/src/uri_test.rs`
- Modify: `crates/nvr-onvif/src/lib.rs`

**Interfaces:**
- Produces: `pub fn inject_credentials(uri: &str, username: &str, password: &str) -> String` — inserts `username:password@` into an RTSP URI's authority (percent-encoding `username`/`password`), leaving a URI that already has credentials, or a non-`rtsp://` URI, or empty credentials, unchanged.

- [ ] **Step 1: Write `uri.rs`**

Create `crates/nvr-onvif/src/uri.rs`:

```rust
/// Inject `username:password@` into an RTSP URI's authority. ONVIF
/// `GetStreamUri` returns a credential-less URI, but many cameras still require
/// RTSP auth with the same credentials, so ffmpeg needs them in the URL.
///
/// Leaves unchanged: a URI that already has `@` in its authority, a non-rtsp
/// scheme, or empty credentials. Percent-encodes the user/pass.
pub fn inject_credentials(uri: &str, username: &str, password: &str) -> String {
    if username.is_empty() || !uri.starts_with("rtsp://") {
        return uri.to_string();
    }
    let rest = &uri["rtsp://".len()..];
    // Already has userinfo (authority contains '@' before the first '/').
    let authority_end = rest.find('/').unwrap_or(rest.len());
    if rest[..authority_end].contains('@') {
        return uri.to_string();
    }
    format!(
        "rtsp://{}:{}@{}",
        encode(username),
        encode(password),
        rest
    )
}

/// Minimal RFC3986 userinfo percent-encoding: keep unreserved chars, encode the
/// rest (notably `:` `@` `/` `?` `#` and anything non-ASCII-alphanumeric).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~');
        if unreserved {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
#[path = "uri_test.rs"]
mod uri_test;
```

- [ ] **Step 2: Export from `lib.rs`**

Add to `crates/nvr-onvif/src/lib.rs`:

```rust
pub mod uri;

pub use uri::inject_credentials;
```

- [ ] **Step 3: Write the failing tests**

Create `crates/nvr-onvif/src/uri_test.rs`:

```rust
use super::*;

#[test]
fn injects_into_plain_rtsp() {
    assert_eq!(
        inject_credentials("rtsp://192.168.1.5:554/Streaming/1", "admin", "pass"),
        "rtsp://admin:pass@192.168.1.5:554/Streaming/1"
    );
}

#[test]
fn percent_encodes_special_chars() {
    assert_eq!(
        inject_credentials("rtsp://cam/live", "adm in", "p@ss:1"),
        "rtsp://adm%20in:p%40ss%3A1@cam/live"
    );
}

#[test]
fn leaves_existing_credentials_untouched() {
    let u = "rtsp://user:pw@cam/live";
    assert_eq!(inject_credentials(u, "admin", "x"), u);
}

#[test]
fn passes_through_empty_user_or_non_rtsp() {
    assert_eq!(inject_credentials("rtsp://cam/live", "", "x"), "rtsp://cam/live");
    assert_eq!(inject_credentials("http://cam/x", "a", "b"), "http://cam/x");
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p nvr-onvif`
Expected: PASS (8 tests total).

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p nvr-onvif
git add crates/nvr-onvif/src
git commit -m "feat(nvr-onvif): RTSP credential injection helper"
```

---

### Task 4: `OnvifCamera` — SOAP client (`camera.rs`)

**Files:**
- Create: `crates/nvr-onvif/src/camera.rs`
- Modify: `crates/nvr-onvif/src/lib.rs`

**Interfaces:**
- Consumes: `OnvifConfig` (Task 1); `DeviceInfo`/`Profile`/`Preset`/`PtzVelocity`/`OnvifError` (Task 2); upstream `onvif`/`schema` crates.
- Produces: `pub struct OnvifCamera` with `async fn connect(cfg: &OnvifConfig) -> Result<OnvifCamera, OnvifError>`, `async fn device_info(&self) -> Result<DeviceInfo, OnvifError>`, `async fn profiles(&self) -> Result<Vec<Profile>, OnvifError>`, `async fn stream_uri(&self, profile: Option<&str>) -> Result<String, OnvifError>`, `async fn ptz_move(&self, v: PtzVelocity) -> Result<(), OnvifError>`, `async fn ptz_stop(&self) -> Result<(), OnvifError>`, `async fn presets(&self) -> Result<Vec<Preset>, OnvifError>`, `async fn goto_preset(&self, token: &str) -> Result<(), OnvifError>`.

> **onvif-rs caveat (see Global Constraints):** the code below is the STARTING SHAPE based on `onvif/examples/camera.rs`. Adapt the upstream struct/field/function names until `cargo build -p nvr-onvif` is clean, mirroring the example, but keep the public method signatures above exactly. Do not change other files. This task has **no unit test** (SOAP needs a live device — validated by Task 6). Its gate is: the crate compiles.

- [ ] **Step 1: Write `camera.rs` (adapt onvif-rs calls until it compiles)**

Create `crates/nvr-onvif/src/camera.rs`:

```rust
use onvif::soap::client::{AuthType, Client, ClientBuilder, Credentials};
use schema::onvif as onvif_xsd;

use crate::config::OnvifConfig;
use crate::types::{DeviceInfo, OnvifError, Preset, Profile, PtzVelocity};

/// A connected ONVIF camera: per-service SOAP clients resolved once at connect.
pub struct OnvifCamera {
    devicemgmt: Client,
    media: Client,
    ptz: Option<Client>,
    /// Fallback profile token (first profile) used when a caller passes None.
    default_profile: String,
}

fn creds(cfg: &OnvifConfig) -> Option<Credentials> {
    if cfg.username.is_empty() {
        None
    } else {
        Some(Credentials {
            username: cfg.username.clone(),
            password: cfg.password.clone(),
        })
    }
}

fn client_at(url: &str, cfg: &OnvifConfig) -> Result<Client, OnvifError> {
    ClientBuilder::new(url)
        .credentials(creds(cfg))
        .auth_type(AuthType::Any)
        .build()
        .map_err(|e| OnvifError::Connect(format!("{e:?}")))
}

fn proto(e: impl std::fmt::Debug) -> OnvifError {
    OnvifError::Protocol(format!("{e:?}"))
}

impl OnvifCamera {
    pub async fn connect(cfg: &OnvifConfig) -> Result<OnvifCamera, OnvifError> {
        let devicemgmt = client_at(&cfg.service_url(), cfg)?;

        // Resolve media/ptz service addresses from GetCapabilities.
        let caps = schema::devicemgmt::get_capabilities(&devicemgmt, &Default::default())
            .await
            .map_err(proto)?;
        let media_addr = caps
            .capabilities
            .media
            .as_ref()
            .map(|m| m.x_addr.clone())
            .ok_or_else(|| OnvifError::Protocol("device advertises no media service".into()))?;
        let media = client_at(&media_addr, cfg)?;
        let ptz = caps
            .capabilities
            .ptz
            .as_ref()
            .map(|p| client_at(&p.x_addr, cfg))
            .transpose()?;

        // First profile token = default.
        let profiles = schema::media::get_profiles(&media, &Default::default())
            .await
            .map_err(proto)?;
        let default_profile = profiles
            .profiles
            .first()
            .map(|p| p.token.0.clone())
            .ok_or_else(|| OnvifError::Protocol("device has no media profiles".into()))?;

        Ok(OnvifCamera { devicemgmt, media, ptz, default_profile })
    }

    pub async fn device_info(&self) -> Result<DeviceInfo, OnvifError> {
        let i = schema::devicemgmt::get_device_information(&self.devicemgmt, &Default::default())
            .await
            .map_err(proto)?;
        Ok(DeviceInfo {
            manufacturer: i.manufacturer,
            model: i.model,
            firmware: i.firmware_version,
            serial: i.serial_number,
        })
    }

    pub async fn profiles(&self) -> Result<Vec<Profile>, OnvifError> {
        let resp = schema::media::get_profiles(&self.media, &Default::default())
            .await
            .map_err(proto)?;
        Ok(resp
            .profiles
            .iter()
            .map(|p| {
                let (width, height, codec) = p
                    .video_encoder_configuration
                    .as_ref()
                    .map(|v| {
                        (
                            v.resolution.width.max(0) as u32,
                            v.resolution.height.max(0) as u32,
                            format!("{:?}", v.encoding),
                        )
                    })
                    .unwrap_or((0, 0, String::new()));
                Profile {
                    token: p.token.0.clone(),
                    name: p.name.0.clone(),
                    width,
                    height,
                    video_codec: codec,
                    fps: 0.0,
                }
            })
            .collect())
    }

    pub async fn stream_uri(&self, profile: Option<&str>) -> Result<String, OnvifError> {
        let token = profile.unwrap_or(&self.default_profile).to_string();
        let req = schema::media::GetStreamUri {
            profile_token: onvif_xsd::ReferenceToken(token),
            stream_setup: onvif_xsd::StreamSetup {
                stream: onvif_xsd::StreamType::RtpUnicast,
                transport: onvif_xsd::Transport {
                    protocol: onvif_xsd::TransportProtocol::Rtsp,
                    tunnel: vec![],
                },
            },
        };
        let resp = schema::media::get_stream_uri(&self.media, &req)
            .await
            .map_err(proto)?;
        Ok(resp.media_uri.uri)
    }

    pub async fn ptz_move(&self, v: PtzVelocity) -> Result<(), OnvifError> {
        let ptz = self.ptz.as_ref().ok_or(OnvifError::NoPtzService)?;
        let req = schema::ptz::ContinuousMove {
            profile_token: onvif_xsd::ReferenceToken(self.default_profile.clone()),
            velocity: Some(onvif_xsd::Ptzspeed {
                pan_tilt: Some(onvif_xsd::Vector2D { x: v.pan, y: v.tilt, space: None }),
                zoom: Some(onvif_xsd::Vector1D { x: v.zoom, space: None }),
            }),
            timeout: None,
        };
        schema::ptz::continuous_move(ptz, &req).await.map_err(proto)?;
        Ok(())
    }

    pub async fn ptz_stop(&self) -> Result<(), OnvifError> {
        let ptz = self.ptz.as_ref().ok_or(OnvifError::NoPtzService)?;
        let req = schema::ptz::Stop {
            profile_token: onvif_xsd::ReferenceToken(self.default_profile.clone()),
            pan_tilt: Some(true),
            zoom: Some(true),
        };
        schema::ptz::stop(ptz, &req).await.map_err(proto)?;
        Ok(())
    }

    pub async fn presets(&self) -> Result<Vec<Preset>, OnvifError> {
        let ptz = self.ptz.as_ref().ok_or(OnvifError::NoPtzService)?;
        let req = schema::ptz::GetPresets {
            profile_token: onvif_xsd::ReferenceToken(self.default_profile.clone()),
        };
        let resp = schema::ptz::get_presets(ptz, &req).await.map_err(proto)?;
        Ok(resp
            .preset
            .iter()
            .filter_map(|p| {
                p.token.as_ref().map(|t| Preset {
                    token: t.0.clone(),
                    name: p.name.as_ref().map(|n| n.0.clone()).unwrap_or_default(),
                })
            })
            .collect())
    }

    pub async fn goto_preset(&self, token: &str) -> Result<(), OnvifError> {
        let ptz = self.ptz.as_ref().ok_or(OnvifError::NoPtzService)?;
        let req = schema::ptz::GotoPreset {
            profile_token: onvif_xsd::ReferenceToken(self.default_profile.clone()),
            preset_token: onvif_xsd::ReferenceToken(token.to_string()),
            speed: None,
        };
        schema::ptz::goto_preset(ptz, &req).await.map_err(proto)?;
        Ok(())
    }
}
```

- [ ] **Step 2: Export from `lib.rs`**

Add to `crates/nvr-onvif/src/lib.rs`:

```rust
pub mod camera;

pub use camera::OnvifCamera;
```

- [ ] **Step 3: Compile (adapt onvif-rs calls until clean)**

Run: `cargo build -p nvr-onvif`
Expected: compiles. If upstream names differ (e.g. `Ptzspeed` vs `PtzSpeed`, field names, `get_capabilities` request type), adjust to match the pinned version by consulting the upstream `onvif/examples/camera.rs` and the `schema` crate — keep the public method signatures unchanged.

- [ ] **Step 4: Verify prior tests still pass**

Run: `cargo test -p nvr-onvif`
Expected: 8 tests pass (no new unit tests this task).

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p nvr-onvif
git add crates/nvr-onvif/src
git commit -m "feat(nvr-onvif): OnvifCamera SOAP client (connect/profiles/stream_uri/ptz)"
```

---

### Task 5: WS-Discovery (`discovery.rs`)

**Files:**
- Create: `crates/nvr-onvif/src/discovery.rs`
- Modify: `crates/nvr-onvif/src/lib.rs`

**Interfaces:**
- Consumes: `Discovered` (Task 2); upstream `onvif::discovery`.
- Produces: `pub async fn discover(timeout: std::time::Duration) -> Result<Vec<Discovered>, OnvifError>` — runs WS-Discovery for `timeout` and maps each found device to `Discovered` (`addr` = host:port parsed from the first endpoint URL).

> **onvif-rs caveat:** starting shape based on `onvif::discovery::DiscoveryBuilder`. Adapt field/method names until it compiles; keep the signature. No unit test (needs network); gate is compile.

- [ ] **Step 1: Write `discovery.rs`**

Create `crates/nvr-onvif/src/discovery.rs`:

```rust
use std::time::Duration;

use futures::StreamExt;

use crate::types::{Discovered, OnvifError};

/// Probe the LAN for ONVIF devices via WS-Discovery for `timeout`.
pub async fn discover(timeout: Duration) -> Result<Vec<Discovered>, OnvifError> {
    let stream = onvif::discovery::DiscoveryBuilder::default()
        .duration(timeout)
        .run()
        .await
        .map_err(|e| OnvifError::Protocol(format!("discovery: {e:?}")))?;

    let devices = stream
        .map(|d| {
            let endpoints: Vec<String> = d.urls.iter().map(|u| u.to_string()).collect();
            let addr = endpoints.first().and_then(|u| {
                url::Url::parse(u).ok().and_then(|parsed| {
                    parsed
                        .host_str()
                        .map(|h| match parsed.port() {
                            Some(p) => format!("{h}:{p}"),
                            None => h.to_string(),
                        })
                })
            });
            Discovered {
                endpoints,
                name: d.name,
                hardware: d.hardware,
                addr,
            }
        })
        .collect::<Vec<_>>()
        .await;

    Ok(devices)
}
```

- [ ] **Step 2: Export from `lib.rs`**

Add to `crates/nvr-onvif/src/lib.rs`:

```rust
pub mod discovery;

pub use discovery::discover;
```

- [ ] **Step 3: Compile (adapt until clean) & verify tests**

Run: `cargo build -p nvr-onvif` then `cargo test -p nvr-onvif`
Expected: compiles; 8 tests still pass. Adapt `DiscoveryBuilder` method names / `Device` field names (`urls`, `name`, `hardware`) to the pinned version if they differ.

- [ ] **Step 4: Format & commit**

```bash
cargo fmt -p nvr-onvif
git add crates/nvr-onvif/src
git commit -m "feat(nvr-onvif): WS-Discovery probe"
```

---

### Task 6: Ignored live integration test (`tests/live.rs`)

**Files:**
- Create: `crates/nvr-onvif/tests/live.rs`

**Interfaces:**
- Consumes: the whole `nvr_onvif` public API.

> `#[ignore]` — needs a real camera / ONVIF simulator. Reads `ONVIF_TEST_HOST` / `ONVIF_TEST_PORT` / `ONVIF_TEST_USER` / `ONVIF_TEST_PASS`; skips (early return) if `ONVIF_TEST_HOST` is unset, so CI never runs the network path.

- [ ] **Step 1: Write the ignored integration test**

Create `crates/nvr-onvif/tests/live.rs`:

```rust
use std::time::Duration;

use nvr_onvif::{OnvifCamera, OnvifConfig, PtzVelocity};

/// End-to-end against a real ONVIF camera / simulator. Run with, e.g.:
///   ONVIF_TEST_HOST=192.168.1.50 ONVIF_TEST_PORT=8000 \
///   ONVIF_TEST_USER=admin ONVIF_TEST_PASS=secret \
///   cargo test -p nvr-onvif --test live -- --ignored --nocapture
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn connect_profiles_stream_uri_ptz() {
    let Ok(host) = std::env::var("ONVIF_TEST_HOST") else {
        eprintln!("ONVIF_TEST_HOST not set; skipping");
        return;
    };
    let cfg = OnvifConfig {
        host,
        port: std::env::var("ONVIF_TEST_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(80),
        username: std::env::var("ONVIF_TEST_USER").unwrap_or_default(),
        password: std::env::var("ONVIF_TEST_PASS").unwrap_or_default(),
        profile_token: None,
    };

    let cam = OnvifCamera::connect(&cfg).await.expect("connect");
    let info = cam.device_info().await.expect("device_info");
    println!("device: {} {} fw {}", info.manufacturer, info.model, info.firmware);

    let profiles = cam.profiles().await.expect("profiles");
    assert!(!profiles.is_empty(), "expected at least one media profile");

    let uri = cam.stream_uri(None).await.expect("stream_uri");
    assert!(uri.starts_with("rtsp://"), "stream uri must be rtsp: {uri}");

    // PTZ is best-effort: some cameras have none. Don't fail the test on NoPtzService.
    if let Err(e) = cam.ptz_move(PtzVelocity::new(0.1, 0.0, 0.0)).await {
        println!("ptz_move skipped: {e}");
    } else {
        tokio::time::sleep(Duration::from_millis(300)).await;
        cam.ptz_stop().await.expect("ptz_stop");
    }
}
```

- [ ] **Step 2: Confirm it compiles and is skipped by default**

Run: `cargo test -p nvr-onvif`
Expected: builds; the live test is listed as `ignored`; 8 unit tests pass.

- [ ] **Step 3: (Manual, optional) run against a real camera / simulator**

If a camera or the ONVIF Device Manager / happytimesoft simulator is reachable, run the command in the test's doc comment and confirm it prints device info + an `rtsp://` URI.

- [ ] **Step 4: Commit**

```bash
cargo fmt -p nvr-onvif
git add crates/nvr-onvif/tests
git commit -m "test(nvr-onvif): ignored live integration test"
```

---

### Task 7: nvr ONVIF registry (`nvr/src/onvif/mod.rs`)

**Files:**
- Create: `nvr/src/onvif/mod.rs`
- Create: `nvr/src/onvif/mod_test.rs`
- Modify: `nvr/src/main.rs` (add `mod onvif;`)
- Modify: `nvr/Cargo.toml` (add `nvr-onvif` dep)

**Interfaces:**
- Consumes: `nvr_onvif::OnvifConfig`.
- Produces: `pub(crate) fn register(device_id: &str, cfg: OnvifConfig)`, `pub(crate) fn get(device_id: &str) -> Option<OnvifConfig>`, `pub(crate) fn remove(device_id: &str)` — over a process-wide `RwLock<HashMap<String, OnvifConfig>>`.

- [ ] **Step 1: Add the crate dependency**

In `nvr/Cargo.toml`, under `[dependencies]`, add:

```toml
nvr-onvif = { path = "../crates/nvr-onvif" }
```

- [ ] **Step 2: Write `mod.rs`**

Create `nvr/src/onvif/mod.rs`:

```rust
//! ONVIF integration: a device_id -> OnvifConfig registry, the REST surface,
//! and the resolve-on-connect ingestion supervisor. Media reuses the existing
//! RTSP -> ZLM device pipeline; ONVIF only resolves the RTSP URI and drives PTZ.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use nvr_onvif::OnvifConfig;

pub mod api;
pub mod ingest;

/// device_id -> connection config, populated when an `onvif` device is added or
/// restored at startup. PTZ and stream re-resolution read from here.
static REGISTRY: LazyLock<RwLock<HashMap<String, OnvifConfig>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub(crate) fn register(device_id: &str, cfg: OnvifConfig) {
    REGISTRY.write().unwrap().insert(device_id.to_string(), cfg);
}

pub(crate) fn get(device_id: &str) -> Option<OnvifConfig> {
    REGISTRY.read().unwrap().get(device_id).cloned()
}

pub(crate) fn remove(device_id: &str) {
    REGISTRY.write().unwrap().remove(device_id);
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod mod_test;
```

- [ ] **Step 3: Add `mod onvif;` to `main.rs`**

In `nvr/src/main.rs`, add `mod onvif;` in the module list (alphabetically, after `mod metrics;` / near the other `mod` lines).

- [ ] **Step 4: Write the failing test**

Create `nvr/src/onvif/mod_test.rs`:

```rust
use super::*;

fn cfg(host: &str) -> OnvifConfig {
    OnvifConfig {
        host: host.into(),
        port: 80,
        username: "u".into(),
        password: "p".into(),
        profile_token: None,
    }
}

#[test]
fn register_get_remove() {
    register("dev-a", cfg("10.0.0.1"));
    assert_eq!(get("dev-a").unwrap().host, "10.0.0.1");
    remove("dev-a");
    assert!(get("dev-a").is_none());
}
```

- [ ] **Step 5: Run tests**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr onvif::mod_test`
Expected: PASS.

- [ ] **Step 6: Format & commit**

```bash
cargo fmt -p nvr
git add nvr/src/onvif/mod.rs nvr/src/onvif/mod_test.rs nvr/src/main.rs nvr/Cargo.toml Cargo.lock
git commit -m "feat(nvr): ONVIF device-config registry"
```

---

### Task 8: `/api/onvif` router + PTZ verb mapping (`nvr/src/onvif/api.rs`)

**Files:**
- Create: `nvr/src/onvif/api.rs`
- Create: `nvr/src/onvif/api_test.rs`
- Modify: `nvr/src/api.rs` (mount the router)

**Interfaces:**
- Consumes: `nvr_onvif::{OnvifCamera, OnvifConfig, discover, PtzVelocity}`; the registry (`super::get`); the handler helpers `ApiJsonResult`/`ok_json`/`ok_empty` (see `nvr/src/handler/mod.rs`).
- Produces: `pub fn onvif_router() -> axum::Router`; pure `pub(crate) fn resolve_ptz(direction: &str, speed: u8, preset_token: Option<&str>) -> Option<PtzAction>` where `pub(crate) enum PtzAction { Move(PtzVelocity), Stop, Preset(String) }` — returns `None` for an unknown verb, and `None` for `preset_call` with no token.

- [ ] **Step 1: Write the failing test for the pure mapping**

Create `nvr/src/onvif/api_test.rs`:

```rust
use super::*;

#[test]
fn maps_direction_verbs_to_actions() {
    assert_eq!(resolve_ptz("stop", 128, None), Some(PtzAction::Stop));
    assert_eq!(
        resolve_ptz("preset_call", 0, Some("P2")),
        Some(PtzAction::Preset("P2".into()))
    );
    // preset_call needs a token
    assert_eq!(resolve_ptz("preset_call", 0, None), None);
    // unknown verb
    assert_eq!(resolve_ptz("wat", 128, None), None);

    // left at speed 255 -> pan -1.0; right -> +1.0; up -> tilt +1.0
    match resolve_ptz("left", 255, None).unwrap() {
        PtzAction::Move(v) => {
            assert!((v.pan + 1.0).abs() < 1e-6);
            assert_eq!(v.tilt, 0.0);
            assert_eq!(v.zoom, 0.0);
        }
        _ => panic!("expected Move"),
    }
    match resolve_ptz("zoom_in", 255, None).unwrap() {
        PtzAction::Move(v) => assert!((v.zoom - 1.0).abs() < 1e-6),
        _ => panic!("expected Move"),
    }
}
```

- [ ] **Step 2: Write `api.rs`**

Create `nvr/src/onvif/api.rs` (`PtzVelocity` derives `PartialEq`, so `PtzAction` can too — the `resolve_ptz` test compares them directly):

```rust
use axum::{
    Json, Router,
    extract::Path,
    routing::{get, post},
};
use nvr_onvif::{Discovered, OnvifCamera, OnvifConfig, Preset, Profile, PtzVelocity, discover};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::handler::{ApiJsonResult, ok_empty, ok_json};

pub fn onvif_router() -> Router {
    Router::new()
        .route("/discover", post(discover_handler))
        .route("/probe", post(probe))
        .route("/ptz", post(ptz))
        .route("/presets/{device_id}", get(presets))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PtzAction {
    Move(PtzVelocity),
    Stop,
    Preset(String),
}

/// Map the gb-style direction verb + speed (0..=255) to a PTZ action.
/// Returns None for an unknown verb or `preset_call` without a token.
pub(crate) fn resolve_ptz(
    direction: &str,
    speed: u8,
    preset_token: Option<&str>,
) -> Option<PtzAction> {
    let s = speed as f32 / 255.0;
    let mv = |pan: f32, tilt: f32, zoom: f32| {
        Some(PtzAction::Move(PtzVelocity::new(pan, tilt, zoom)))
    };
    match direction {
        "up" => mv(0.0, s, 0.0),
        "down" => mv(0.0, -s, 0.0),
        "left" => mv(-s, 0.0, 0.0),
        "right" => mv(s, 0.0, 0.0),
        "zoom_in" => mv(0.0, 0.0, s),
        "zoom_out" => mv(0.0, 0.0, -s),
        "stop" => Some(PtzAction::Stop),
        "preset_call" => preset_token.map(|t| PtzAction::Preset(t.to_string())),
        _ => None,
    }
}

#[derive(Deserialize)]
struct DiscoverRequest {
    timeout_ms: Option<u64>,
}

async fn discover_handler(
    Json(req): Json<DiscoverRequest>,
) -> ApiJsonResult<Vec<Discovered>> {
    let timeout = Duration::from_millis(req.timeout_ms.unwrap_or(3000).clamp(500, 10_000));
    let found = discover(timeout)
        .await
        .map_err(|e| anyhow::anyhow!("onvif discover: {e}"))?;
    Ok(ok_json(found))
}

#[derive(Deserialize)]
struct ProbeRequest {
    host: String,
    port: u16,
    username: String,
    password: String,
}

#[derive(Serialize)]
struct ProbeResponse {
    device_info: nvr_onvif::DeviceInfo,
    profiles: Vec<Profile>,
}

async fn probe(Json(req): Json<ProbeRequest>) -> ApiJsonResult<ProbeResponse> {
    let cfg = OnvifConfig {
        host: req.host,
        port: req.port,
        username: req.username,
        password: req.password,
        profile_token: None,
    };
    let cam = OnvifCamera::connect(&cfg)
        .await
        .map_err(|e| anyhow::anyhow!("onvif connect: {e}"))?;
    let device_info = cam
        .device_info()
        .await
        .map_err(|e| anyhow::anyhow!("onvif device_info: {e}"))?;
    let profiles = cam
        .profiles()
        .await
        .map_err(|e| anyhow::anyhow!("onvif profiles: {e}"))?;
    Ok(ok_json(ProbeResponse { device_info, profiles }))
}

#[derive(Deserialize)]
struct PtzRequest {
    device_id: String,
    direction: String,
    speed: Option<u8>,
    preset_token: Option<String>,
}

async fn ptz(Json(req): Json<PtzRequest>) -> ApiJsonResult<()> {
    let cfg = super::get(&req.device_id)
        .ok_or_else(|| anyhow::anyhow!("no onvif device: {}", req.device_id))?;
    let action = resolve_ptz(
        &req.direction,
        req.speed.unwrap_or(128),
        req.preset_token.as_deref(),
    )
    .ok_or_else(|| anyhow::anyhow!("bad ptz direction: {}", req.direction))?;

    let cam = OnvifCamera::connect(&cfg)
        .await
        .map_err(|e| anyhow::anyhow!("onvif connect: {e}"))?;
    match action {
        PtzAction::Move(v) => cam.ptz_move(v).await,
        PtzAction::Stop => cam.ptz_stop().await,
        PtzAction::Preset(t) => cam.goto_preset(&t).await,
    }
    .map_err(|e| anyhow::anyhow!("onvif ptz: {e}"))?;
    Ok(ok_empty())
}

async fn presets(Path(device_id): Path<String>) -> ApiJsonResult<Vec<Preset>> {
    let cfg = super::get(&device_id)
        .ok_or_else(|| anyhow::anyhow!("no onvif device: {device_id}"))?;
    let cam = OnvifCamera::connect(&cfg)
        .await
        .map_err(|e| anyhow::anyhow!("onvif connect: {e}"))?;
    let presets = cam
        .presets()
        .await
        .map_err(|e| anyhow::anyhow!("onvif presets: {e}"))?;
    Ok(ok_json(presets))
}

#[cfg(test)]
#[path = "api_test.rs"]
mod api_test;
```

> Verify `ApiJsonResult`/`ok_json`/`ok_empty` names against `nvr/src/handler/mod.rs` and adjust the import if they differ. `probe`/`ptz`/`presets` each `connect` a fresh `OnvifCamera` per call — fine for control-plane operations (they're infrequent); no connection pooling in v1.

- [ ] **Step 3: Mount the router**

In `nvr/src/api.rs`, add alongside the other `.nest(...)` calls (inside the `/api` router that carries the auth layer):

```rust
            .nest("/onvif", crate::onvif::api::onvif_router())
```

- [ ] **Step 4: Run the mapping test + compile**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr onvif::api_test`
Expected: the `maps_direction_verbs_to_actions` test PASSES and `nvr` compiles.

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p nvr
git add nvr/src/onvif/api.rs nvr/src/onvif/api_test.rs nvr/src/api.rs
git commit -m "feat(nvr): /api/onvif router (discover/probe/ptz/presets) + PTZ verb mapping"
```

---

### Task 9: ONVIF ingestion supervisor (`nvr/src/onvif/ingest.rs`)

**Files:**
- Create: `nvr/src/onvif/ingest.rs`
- Modify: `nvr/src/init/device.rs` (add the `input_type == "onvif"` branch)

**Interfaces:**
- Consumes: `nvr_onvif::{OnvifCamera, OnvifConfig, inject_credentials}`; the existing device pipeline used by `livestream::spawn_stream_device` (study `nvr/src/livestream.rs` and `nvr/src/manager.rs` for `run_session` / `Entry::Task` / the `rszlm::media::Media` handoff).
- Produces: `pub(crate) fn spawn_onvif_device(device_id: String, cfg: OnvifConfig, media: std::sync::Arc<rszlm::media::Media>, include_audio: bool, cancel: tokio_util::sync::CancellationToken) -> tokio::task::JoinHandle<()>` — the resolve→run→backoff→re-resolve supervisor.

> This is an integration task modeled directly on `nvr/src/livestream.rs::spawn_stream_device`. The gate is: `nvr` compiles and existing tests pass. Runtime is validated manually / end-to-end. **Read `nvr/src/livestream.rs` first** and mirror its structure exactly, swapping the yt-dlp `resolve` for the ONVIF resolve below.

- [ ] **Step 1: Study the existing supervisor**

Read `nvr/src/livestream.rs` (the `spawn_stream_device` loop: resolve → `run_session` → backoff with `BACKOFF_MIN`/`BACKOFF_MAX`/`HEALTHY_SESSION`) and note the exact `run_session` signature and how `media`/`include_audio`/`cancel` are threaded. `spawn_onvif_device` reuses the SAME `run_session` — only the resolve step changes.

- [ ] **Step 2: Write `ingest.rs`**

Create `nvr/src/onvif/ingest.rs`, mirroring `livestream.rs` structure. The resolve step is:

```rust
// Inside the supervisor loop, replacing yt-dlp resolve with ONVIF resolve:
//   let cam = OnvifCamera::connect(&cfg).await?;
//   let uri = cam.stream_uri(cfg.profile_token.as_deref()).await?;
//   let rtsp = nvr_onvif::inject_credentials(&uri, &cfg.username, &cfg.password);
//   run_session(&rtsp, Arc::clone(&media), include_audio, &cancel).await;
```

Concretely (adapt `run_session`'s real signature from `livestream.rs`):

```rust
use std::sync::Arc;
use std::time::{Duration, Instant};

use nvr_onvif::{OnvifCamera, OnvifConfig, inject_credentials};
use tokio_util::sync::CancellationToken;

const BACKOFF_MIN: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
const HEALTHY_SESSION: Duration = Duration::from_secs(30);

/// Resolve the RTSP URI from ONVIF, pull it into `media`, and re-resolve on
/// reconnect — same shape as `livestream::spawn_stream_device`, so a camera
/// IP/credential change or reboot self-heals.
pub(crate) fn spawn_onvif_device(
    device_id: String,
    cfg: OnvifConfig,
    media: Arc<rszlm::media::Media>,
    include_audio: bool,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = BACKOFF_MIN;
        loop {
            if cancel.is_cancelled() {
                break;
            }
            match resolve_rtsp(&cfg).await {
                Ok(rtsp) => {
                    log::info!("onvif {device_id}: resolved rtsp uri");
                    let started = Instant::now();
                    // NOTE: use the SAME run_session helper livestream.rs uses.
                    crate::livestream::run_session(
                        &rtsp,
                        Arc::clone(&media),
                        include_audio,
                        &cancel,
                    )
                    .await;
                    if cancel.is_cancelled() {
                        break;
                    }
                    if started.elapsed() >= HEALTHY_SESSION {
                        backoff = BACKOFF_MIN;
                    }
                    log::warn!("onvif {device_id}: session ended, re-resolving in {backoff:?}");
                }
                Err(e) => {
                    log::warn!("onvif {device_id}: resolve failed: {e}, retry in {backoff:?}");
                }
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
    })
}

async fn resolve_rtsp(cfg: &OnvifConfig) -> anyhow::Result<String> {
    let cam = OnvifCamera::connect(cfg).await?;
    let uri = cam.stream_uri(cfg.profile_token.as_deref()).await?;
    Ok(inject_credentials(&uri, &cfg.username, &cfg.password))
}
```

> If `livestream::run_session` is private, either make it `pub(crate)` (a one-word change in `livestream.rs`, justified because ONVIF now shares it) or copy its small body. Prefer making it `pub(crate)`. Match its real signature exactly.

- [ ] **Step 3: Wire the `onvif` device branch in `init/device.rs`**

In `nvr/src/init/device.rs::ensure_device_pipe`, add a branch mirroring the `"stream"` branch (which builds a ZLM `Media` and spawns a supervisor `Task`). After the `"gb28181"` branch, add:

```rust
    if device.input_type == "onvif" {
        let cfg: nvr_onvif::OnvifConfig = serde_json::from_str(&device.input_value)
            .map_err(|e| anyhow::anyhow!("invalid onvif device config: {e}"))?;
        crate::onvif::register(&device.id, cfg.clone());
        let media = Arc::new(rszlm::media::Media::new_with_default_vhost(
            DEVICE_APP,
            device.id.as_str(),
            0.0,
            device.record,
            false,
        ));
        // Register the supervisor as a manager Task (mirror how `stream` does it,
        // e.g. manager::upsert_onvif or the same helper `stream` uses). Study
        // how spawn_stream_device's JoinHandle/CancellationToken is registered in
        // the manager and follow the same path so remove_device stops it.
        return crate::manager::upsert_onvif(
            &device.id,
            media,
            cfg,
            device.include_audio,
            true,
        )
        .await;
    }
```

> Follow the exact manager registration `stream` uses (look at `manager::upsert_stream` and add a parallel `upsert_onvif` that calls `crate::onvif::ingest::spawn_onvif_device` and stores the resulting task under the device id, so `remove_device` cancels it and `crate::onvif::remove` clears the registry). Keep the manager change minimal and symmetric with `upsert_stream`.

- [ ] **Step 4: Compile & run existing tests**

Run: `LD_LIBRARY_PATH=$PWD/ffmpeg/lib cargo test -p nvr`
Expected: `nvr` compiles; all existing tests pass (this task adds no unit test — the supervisor is validated end-to-end).

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p nvr
git add nvr/src/onvif/ingest.rs nvr/src/init/device.rs nvr/src/manager.rs nvr/src/livestream.rs
git commit -m "feat(nvr): ONVIF ingestion supervisor (resolve-on-connect RTSP)"
```

---

### Task 10: Dashboard — ONVIF device form (`api/onvif.ts` + DeviceListView)

**Files:**
- Create: `nvr-dashboard/app/src/api/onvif.ts`
- Modify: `nvr-dashboard/app/src/views/DeviceListView.vue`

**Interfaces:**
- Consumes: the shared `request` wrapper (`nvr-dashboard/app/src/api/request.ts`).
- Produces: `discoverOnvif`, `probeOnvif`, `onvifPtz`, `getOnvifPresets` functions + the `OnvifDiscovered`/`OnvifProbe`/`OnvifProfile`/`OnvifPreset` types.

- [ ] **Step 1: Write the API client**

Create `nvr-dashboard/app/src/api/onvif.ts`:

```ts
import { request } from './request'

export interface OnvifDiscovered {
  endpoints: string[]
  name: string | null
  hardware: string | null
  addr: string | null
}
export interface OnvifProfile {
  token: string
  name: string
  width: number
  height: number
  video_codec: string
  fps: number
}
export interface OnvifDeviceInfo {
  manufacturer: string
  model: string
  firmware: string
  serial: string
}
export interface OnvifProbe {
  device_info: OnvifDeviceInfo
  profiles: OnvifProfile[]
}
export interface OnvifPreset {
  token: string
  name: string
}

export function discoverOnvif(timeoutMs = 3000) {
  return request<OnvifDiscovered[]>('/onvif/discover', {
    method: 'POST',
    body: { timeout_ms: timeoutMs },
  })
}

export function probeOnvif(payload: {
  host: string
  port: number
  username: string
  password: string
}) {
  return request<OnvifProbe>('/onvif/probe', { method: 'POST', body: payload })
}

export function onvifPtz(payload: {
  device_id: string
  direction: string
  speed?: number
  preset_token?: string
}) {
  return request<null>('/onvif/ptz', { method: 'POST', body: payload })
}

export function getOnvifPresets(deviceId: string) {
  return request<OnvifPreset[]>(`/onvif/presets/${encodeURIComponent(deviceId)}`)
}
```

- [ ] **Step 2: Add the ONVIF option + form to DeviceListView**

In `nvr-dashboard/app/src/views/DeviceListView.vue`:

1. Add to `inputTypeOptions` (near the existing `gb28181` entry):

```ts
  { label: "ONVIF 摄像头", value: "onvif" },
```

2. When `input_type === "onvif"` is selected, render host / port / username / password inputs, a **「探测」** button that calls `probeOnvif(...)` and shows `device_info` + a profile `Select` (options from `probe.profiles`, `optionLabel` = name+resolution, `optionValue` = token), and an optional **「扫描局域网」** button that calls `discoverOnvif()` and lists results (clicking one prefills host/port parsed from `addr`).
3. On submit for an `onvif` device, set `input_value` to the JSON string of `{ host, port, username, password, profile_token }` (mirror how the `xiaomi` branch builds `input_value` via `JSON.stringify`).

> Follow the existing xiaomi/gb form patterns already in this file for field layout, validation, and how `input_value` is assembled. Use PrimeVue components (`InputText`, `InputNumber`, `Password`, `Select`, `Button`) consistent with the rest of the view.

- [ ] **Step 3: Type-check & lint**

Run: `cd nvr-dashboard/app && npm run type-check && npm run lint`
Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add nvr-dashboard/app/src/api/onvif.ts nvr-dashboard/app/src/views/DeviceListView.vue
git commit -m "feat(dashboard): ONVIF device form with probe and LAN discovery"
```

---

### Task 11: Dashboard — PTZ controls for ONVIF rows

**Files:**
- Modify: `nvr-dashboard/app/src/views/DeviceListView.vue`

**Interfaces:**
- Consumes: `onvifPtz`, `getOnvifPresets` from `api/onvif.ts` (Task 10).

- [ ] **Step 1: Add PTZ controls to ONVIF device rows**

In `DeviceListView.vue`, for rows where `input_type === "onvif"`, render PTZ controls reusing the same layout/component the gb28181 rows use (direction pad: up/down/left/right, zoom in/out, stop; a presets dropdown). Wire them:

- direction button press → `onvifPtz({ device_id, direction, speed: 128 })`; release / stop button → `onvifPtz({ device_id, direction: 'stop' })`.
- presets dropdown: on open, `getOnvifPresets(device_id)` → list; selecting one → `onvifPtz({ device_id, direction: 'preset_call', preset_token })`.

> Reuse the existing gb PTZ control markup/handlers in this file (same direction verbs); only the API call target changes from the gb ptz function to `onvifPtz`. If the gb PTZ controls are a shared component, render it for onvif rows with the onvif handler injected; otherwise copy the gb row's PTZ block and swap the call.

- [ ] **Step 2: Type-check & lint**

Run: `cd nvr-dashboard/app && npm run type-check && npm run lint`
Expected: no errors.

- [ ] **Step 3: Commit**

```bash
git add nvr-dashboard/app/src/views/DeviceListView.vue
git commit -m "feat(dashboard): PTZ controls for ONVIF devices"
```

---

## Notes for the implementer

- **Task 1 is the dependency go/no-go.** If `onvif`/`schema` (git) won't resolve or compile in this workspace, stop and surface it — the whole plan rests on it. Watch for a `reqwest`/`tokio` version split with the workspace (allowed but noisy) and for `links=`-style native conflicts (there should be none — onvif-rs is pure Rust).
- **onvif-rs API drift:** Tasks 4/5 (and the schema types they use) are the most likely to need adjustment against the pinned commit. Keep the public method signatures fixed; adapt only the internal upstream calls, mirroring `onvif/examples/camera.rs`. The `cargo build` in each of those tasks is the real gate.
- **No new media path:** ingestion MUST reuse `livestream::run_session` (or the exact helper the `stream` type uses) — do not write a second RTSP puller.
- **Manager symmetry:** the `onvif` device lifecycle (add → supervisor Task → `remove_device` cancels it + clears the registry) must mirror the `stream` type's manager wiring; keep the `upsert_onvif` change minimal and parallel to `upsert_stream`.
