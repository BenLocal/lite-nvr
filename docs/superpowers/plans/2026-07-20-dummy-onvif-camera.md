# Dummy ONVIF Camera Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `examples/dummy-onvif-camera` — a self-contained simulated ONVIF device that answers exactly the SOAP operations + WS-Discovery `nvr-onvif` calls, reusing `dummy-rtsp-camera` for the actual RTSP stream, so ONVIF discovery → probe → RTSP ingestion → PTZ can be verified on one machine.

**Architecture:** A Tokio binary running three parts: an axum HTTP server at `/onvif/device_service` that authenticates (WS-Security UsernameToken) and returns canned SOAP responses for the 8 operations; a UDP WS-Discovery responder (multicast `Probe` → unicast `ProbeMatches`); and the RTSP plane delegated to `dummy-rtsp-camera` (default URL, optional child spawn). All ONVIF/SOAP is hand-served (no Rust ONVIF server lib exists); the response XML is validated against the real `nvr-onvif` client end-to-end.

**Tech Stack:** Rust edition 2024, `tokio`, `axum` 0.8, `quick-xml` 0.37, `sha1` 0.10 + `base64` 0.22 (WS-Security digest), `uuid`, `clap`, `env_logger`/`log`, `which`, `socket2` (UDP multicast).

## Global Constraints

- Rust **edition 2024**; `snake_case`; run `cargo fmt -p dummy-onvif-camera` before every commit.
- New binary crate `examples/dummy-onvif-camera` (added to root `Cargo.toml` `members`), mirroring `examples/dummy-rtsp-camera`. It is a standalone example — do NOT modify `crates/*`, `nvr`, or the dashboard.
- **No new media path / no ONVIF server dependency:** the RTSP stream is `dummy-rtsp-camera` (oddity); ONVIF SOAP is hand-served in this crate.
- Tests colocated as `<module>_test.rs`, imported via `#[cfg(test)] #[path = "<module>_test.rs"] mod <module>_test;` (repo convention in `CLAUDE.md`).
- This crate is **pure Rust (no ffmpeg)** → `cargo test -p dummy-onvif-camera` needs no `LD_LIBRARY_PATH`.
- **Auth-reject contract (verified against pinned `onvif-rs` `8f1490e`, `onvif/src/soap/client.rs:307-316` + `xsd_rs/soap_envelope/src/lib.rs:74`):** the client yields `transport::Error::Authorization` (→ `OnvifError::Auth`) when a **non-2xx** response body parses as a SOAP `Fault` whose `Code/Subcode/Value` **contains the substring `"NotAuthorized"`**. So a bad/missing UsernameToken → **HTTP 400 + a `ter:NotAuthorized` SOAP fault** (NOT HTTP 401, which triggers a digest handshake).
- **ONVIF namespaces used in every response** (yaserde in the `schema` crate is namespace-strict):
  - `env` = `http://www.w3.org/2003/05/soap-envelope` (SOAP 1.2)
  - `tds` = `http://www.onvif.org/ver10/device/wsdl`
  - `trt` = `http://www.onvif.org/ver10/media/wsdl`
  - `tptz` = `http://www.onvif.org/ver20/ptz/wsdl`
  - `tt` = `http://www.onvif.org/ver10/schema`
  - `ter` = `http://www.onvif.org/ver10/error`
- **Response XML is best-effort** (based on the ONVIF spec + `schema` crate element names). Task 7 runs the real `nvr-onvif` client against the dummy and adapts the templates until it parses — the E2E run is the definitive gate for XML correctness. Keep the pure function *signatures* stable; only the XML *string contents* may change in Task 7.

---

### Task 1: Scaffold crate + `DeviceCfg` + CLI

**Files:**
- Create: `examples/dummy-onvif-camera/Cargo.toml`
- Create: `examples/dummy-onvif-camera/src/main.rs`
- Create: `examples/dummy-onvif-camera/src/config.rs`
- Create: `examples/dummy-onvif-camera/src/config_test.rs`
- Modify: `Cargo.toml` (workspace `members`)

**Interfaces:**
- Produces: `DeviceCfg { host: String, port: u16, username: String, password: String, rtsp_url: String, manufacturer: String, model: String, firmware: String, serial: String }` (`#[derive(Clone, Debug)]`) with `DeviceCfg::service_url(&self) -> String` = `http://{host}:{port}/onvif/device_service`; and a clap `Args` struct with `Args::into_cfg(self) -> (DeviceCfg, RuntimeOpts)` where `RuntimeOpts { launch_rtsp: bool, discovery: bool }`.

- [ ] **Step 1: Create the manifest**

Create `examples/dummy-onvif-camera/Cargo.toml`:

```toml
[package]
name = "dummy-onvif-camera"
version = "0.1.0"
edition = "2024"
publish = false
description = "A simulated ONVIF camera: hand-served SOAP + WS-Discovery, reusing dummy-rtsp-camera for RTSP. Test fixture for nvr-onvif."

[dependencies]
tokio = { workspace = true, features = ["full"] }
axum = { workspace = true }
anyhow = { workspace = true }
log = { workspace = true }
env_logger = { workspace = true }
quick-xml = { workspace = true }
base64 = { workspace = true }
sha1 = { workspace = true }
uuid = { workspace = true, features = ["v4"] }
which = { workspace = true }
clap = { version = "4", features = ["derive"] }
socket2 = "0.5"
```

- [ ] **Step 2: Register in the workspace**

In root `Cargo.toml`, add to `members` (after `"examples/dummy-rtsp-camera",`):

```toml
    "examples/dummy-rtsp-camera",
    "examples/dummy-onvif-camera",
```

- [ ] **Step 3: Write `config.rs`**

Create `examples/dummy-onvif-camera/src/config.rs`:

```rust
use clap::Parser;

/// Everything the SOAP responses and discovery need to describe this device.
#[derive(Clone, Debug)]
pub struct DeviceCfg {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub rtsp_url: String,
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    pub serial: String,
}

impl DeviceCfg {
    /// The device-management service URL (also advertised for media + ptz).
    pub fn service_url(&self) -> String {
        format!("http://{}:{}/onvif/device_service", self.host, self.port)
    }
}

/// Runtime toggles that aren't part of the device description.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeOpts {
    pub launch_rtsp: bool,
    pub discovery: bool,
}

#[derive(Parser, Debug)]
#[command(name = "dummy-onvif-camera", about = "Simulated ONVIF camera for testing nvr-onvif.")]
pub struct Args {
    /// Advertised media/service IP.
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,
    /// ONVIF HTTP port.
    #[arg(long, default_value_t = 8000)]
    pub port: u16,
    /// Required WS-Security username.
    #[arg(long, default_value = "admin")]
    pub username: String,
    /// Required WS-Security password.
    #[arg(long, default_value = "admin")]
    pub password: String,
    /// URL returned by GetStreamUri.
    #[arg(long, default_value = "rtsp://127.0.0.1:9554/live/test1")]
    pub rtsp_url: String,
    /// Also spawn dummy-rtsp-camera as a child.
    #[arg(long, default_value_t = false)]
    pub launch_rtsp: bool,
    /// Disable the WS-Discovery responder (it runs by default).
    #[arg(long, default_value_t = false)]
    pub no_discovery: bool,
    #[arg(long, default_value = "lite-nvr")]
    pub manufacturer: String,
    #[arg(long, default_value = "dummy-onvif-camera")]
    pub model: String,
    #[arg(long, default_value = "0.1")]
    pub firmware: String,
    #[arg(long, default_value = "SN-0001")]
    pub serial: String,
}

impl Args {
    pub fn into_cfg(self) -> (DeviceCfg, RuntimeOpts) {
        let opts = RuntimeOpts { launch_rtsp: self.launch_rtsp, discovery: !self.no_discovery };
        let cfg = DeviceCfg {
            host: self.host,
            port: self.port,
            username: self.username,
            password: self.password,
            rtsp_url: self.rtsp_url,
            manufacturer: self.manufacturer,
            model: self.model,
            firmware: self.firmware,
            serial: self.serial,
        };
        (cfg, opts)
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
```

- [ ] **Step 4: Write a minimal `main.rs` (compiles; wired up in Task 6)**

Create `examples/dummy-onvif-camera/src/main.rs`:

```rust
mod config;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    let (cfg, opts) = config::Args::parse().into_cfg();
    log::info!(
        "dummy-onvif-camera: service {} (rtsp {}, discovery {}, launch_rtsp {})",
        cfg.service_url(),
        cfg.rtsp_url,
        opts.discovery,
        opts.launch_rtsp
    );
    // Server wiring lands in Task 6.
    Ok(())
}
```

- [ ] **Step 5: Write the failing test**

Create `examples/dummy-onvif-camera/src/config_test.rs`:

```rust
use super::*;

#[test]
fn service_url_format() {
    let (cfg, _) = Args::parse_from(["x", "--host", "10.0.0.9", "--port", "8899"]).into_cfg();
    assert_eq!(cfg.service_url(), "http://10.0.0.9:8899/onvif/device_service");
}

#[test]
fn defaults_and_toggles() {
    let (cfg, opts) = Args::parse_from(["x"]).into_cfg();
    assert_eq!(cfg.host, "127.0.0.1");
    assert_eq!(cfg.port, 8000);
    assert_eq!(cfg.username, "admin");
    assert_eq!(cfg.password, "admin");
    assert_eq!(cfg.rtsp_url, "rtsp://127.0.0.1:9554/live/test1");
    assert!(opts.discovery); // on unless --no-discovery
    assert!(!opts.launch_rtsp);

    let (_, opts2) = Args::parse_from(["x", "--no-discovery", "--launch-rtsp"]).into_cfg();
    assert!(!opts2.discovery);
    assert!(opts2.launch_rtsp);
}
```

`Args::parse_from` needs `use clap::Parser;` — it's brought in via `use super::*;` since `config.rs` has `use clap::Parser;`. If not in scope, add `use clap::Parser;` to the test.

- [ ] **Step 6: Run tests**

Run: `cargo test -p dummy-onvif-camera`
Expected: PASS (2 tests); the binary builds.

- [ ] **Step 7: Format & commit**

```bash
cargo fmt -p dummy-onvif-camera
git add examples/dummy-onvif-camera Cargo.toml Cargo.lock
git commit -m "feat(dummy-onvif-camera): scaffold crate, DeviceCfg + CLI"
```

---

### Task 2: WS-Security UsernameToken verification (`auth.rs`)

**Files:**
- Create: `examples/dummy-onvif-camera/src/auth.rs`
- Create: `examples/dummy-onvif-camera/src/auth_test.rs`
- Modify: `examples/dummy-onvif-camera/src/main.rs` (add `mod auth;`)

**Interfaces:**
- Produces: `pub struct UsernameToken { pub username: String, pub password_digest: String, pub nonce: String, pub created: String }`; `pub fn verify(token: &UsernameToken, cfg_user: &str, cfg_pass: &str) -> bool` — true iff `token.username == cfg_user` AND `token.password_digest == base64(sha1(base64_decode(token.nonce) ++ token.created.as_bytes() ++ cfg_pass.as_bytes()))`. Also `pub fn password_digest(nonce_b64: &str, created: &str, password: &str) -> String` (the same computation, exposed so tests and callers agree).

- [ ] **Step 1: Write `auth.rs`**

Create `examples/dummy-onvif-camera/src/auth.rs`:

```rust
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use sha1::{Digest, Sha1};

/// The fields extracted from a WS-Security UsernameToken (PasswordDigest mode).
#[derive(Clone, Debug, PartialEq)]
pub struct UsernameToken {
    pub username: String,
    pub password_digest: String,
    pub nonce: String,   // base64 as it appears in the XML
    pub created: String, // the UTC timestamp string, verbatim
}

/// PasswordDigest = Base64( SHA1( decode(nonce) ++ created ++ password ) ).
pub fn password_digest(nonce_b64: &str, created: &str, password: &str) -> String {
    let nonce = B64.decode(nonce_b64).unwrap_or_default();
    let mut h = Sha1::new();
    h.update(&nonce);
    h.update(created.as_bytes());
    h.update(password.as_bytes());
    B64.encode(h.finalize())
}

/// True iff the token's username matches and its digest recomputes correctly.
pub fn verify(token: &UsernameToken, cfg_user: &str, cfg_pass: &str) -> bool {
    token.username == cfg_user
        && token.password_digest == password_digest(&token.nonce, &token.created, cfg_pass)
}

#[cfg(test)]
#[path = "auth_test.rs"]
mod auth_test;
```

- [ ] **Step 2: Add `mod auth;` to `main.rs`**

In `examples/dummy-onvif-camera/src/main.rs`, add below `mod config;`:

```rust
mod auth;
```

- [ ] **Step 3: Write the failing tests**

Create `examples/dummy-onvif-camera/src/auth_test.rs`:

```rust
use super::*;

fn token(user: &str, nonce: &str, created: &str, pass: &str) -> UsernameToken {
    UsernameToken {
        username: user.to_string(),
        password_digest: password_digest(nonce, created, pass),
        nonce: nonce.to_string(),
        created: created.to_string(),
    }
}

#[test]
fn accepts_correct_credentials() {
    // nonce is base64 of some bytes; created is any string.
    let t = token("admin", "MTIzNDU2Nzg5MDEyMzQ1Ng==", "2026-07-20T00:00:00Z", "secret");
    assert!(verify(&t, "admin", "secret"));
}

#[test]
fn rejects_wrong_password() {
    let t = token("admin", "MTIzNDU2Nzg5MDEyMzQ1Ng==", "2026-07-20T00:00:00Z", "secret");
    assert!(!verify(&t, "admin", "WRONG"));
}

#[test]
fn rejects_wrong_username() {
    let t = token("admin", "MTIzNDU2Nzg5MDEyMzQ1Ng==", "2026-07-20T00:00:00Z", "secret");
    assert!(!verify(&t, "root", "secret"));
}

#[test]
fn digest_is_deterministic_and_known() {
    // SHA1("" nonce-bytes are empty for empty b64 || "C" || "P") sanity: same inputs -> same digest.
    let a = password_digest("", "C", "P");
    let b = password_digest("", "C", "P");
    assert_eq!(a, b);
    assert!(!a.is_empty());
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p dummy-onvif-camera auth`
Expected: PASS (4 auth tests).

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p dummy-onvif-camera
git add examples/dummy-onvif-camera/src
git commit -m "feat(dummy-onvif-camera): WS-Security UsernameToken verify"
```

---

### Task 3: SOAP response builders (`responses.rs`)

**Files:**
- Create: `examples/dummy-onvif-camera/src/responses.rs`
- Create: `examples/dummy-onvif-camera/src/responses_test.rs`
- Modify: `examples/dummy-onvif-camera/src/main.rs` (add `mod responses;`)

**Interfaces:**
- Consumes: `DeviceCfg` (Task 1).
- Produces (all `pub fn ... -> String`): `get_capabilities(service_url: &str)`, `device_information(cfg: &DeviceCfg)`, `get_profiles()`, `get_stream_uri(rtsp_url: &str)`, `get_presets()`, `ptz_ack(op_response_element: &str)`, `fault_not_authorized()`, `fault_action_not_supported()`. Each returns a complete SOAP 1.2 envelope string. A shared `pub(crate) fn envelope(body: &str) -> String` wraps a body element in the namespaced envelope.

> The XML below is the **best-effort starting shape** (ONVIF-spec element names + the namespaces in Global Constraints). Task 7 adapts the exact element/namespace details until the real `nvr-onvif` client parses them. Keep these function signatures stable.

- [ ] **Step 1: Write `responses.rs`**

Create `examples/dummy-onvif-camera/src/responses.rs`:

```rust
use crate::config::DeviceCfg;

const NS: &str = concat!(
    r#" xmlns:env="http://www.w3.org/2003/05/soap-envelope""#,
    r#" xmlns:tds="http://www.onvif.org/ver10/device/wsdl""#,
    r#" xmlns:trt="http://www.onvif.org/ver10/media/wsdl""#,
    r#" xmlns:tptz="http://www.onvif.org/ver20/ptz/wsdl""#,
    r#" xmlns:tt="http://www.onvif.org/ver10/schema""#,
    r#" xmlns:ter="http://www.onvif.org/ver10/error""#,
);

/// Wrap a body-inner XML fragment in a namespaced SOAP 1.2 envelope.
pub(crate) fn envelope(body_inner: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><env:Envelope{NS}><env:Body>{body_inner}</env:Body></env:Envelope>"#
    )
}

pub fn get_capabilities(service_url: &str) -> String {
    envelope(&format!(
        r#"<tds:GetCapabilitiesResponse><tds:Capabilities>
<tt:Media><tt:XAddr>{service_url}</tt:XAddr></tt:Media>
<tt:PTZ><tt:XAddr>{service_url}</tt:XAddr></tt:PTZ>
</tds:Capabilities></tds:GetCapabilitiesResponse>"#
    ))
}

pub fn device_information(cfg: &DeviceCfg) -> String {
    envelope(&format!(
        r#"<tds:GetDeviceInformationResponse>
<tds:Manufacturer>{m}</tds:Manufacturer>
<tds:Model>{mo}</tds:Model>
<tds:FirmwareVersion>{f}</tds:FirmwareVersion>
<tds:SerialNumber>{s}</tds:SerialNumber>
<tds:HardwareId>HW-1</tds:HardwareId>
</tds:GetDeviceInformationResponse>"#,
        m = cfg.manufacturer, mo = cfg.model, f = cfg.firmware, s = cfg.serial
    ))
}

pub fn get_profiles() -> String {
    envelope(
        r#"<trt:GetProfilesResponse><trt:Profiles token="Profile_1" fixed="true">
<tt:Name>Profile_1</tt:Name>
<tt:VideoEncoderConfiguration token="VEC_1">
<tt:Name>VEC_1</tt:Name><tt:Encoding>H264</tt:Encoding>
<tt:Resolution><tt:Width>1920</tt:Width><tt:Height>1080</tt:Height></tt:Resolution>
</tt:VideoEncoderConfiguration>
</trt:Profiles></trt:GetProfilesResponse>"#,
    )
}

pub fn get_stream_uri(rtsp_url: &str) -> String {
    envelope(&format!(
        r#"<trt:GetStreamUriResponse><trt:MediaUri>
<tt:Uri>{rtsp_url}</tt:Uri>
<tt:InvalidAfterConnect>false</tt:InvalidAfterConnect>
<tt:InvalidAfterReboot>false</tt:InvalidAfterReboot>
<tt:Timeout>PT60S</tt:Timeout>
</trt:MediaUri></trt:GetStreamUriResponse>"#
    ))
}

pub fn get_presets() -> String {
    envelope(
        r#"<tptz:GetPresetsResponse>
<tptz:Preset token="Preset_1"><tt:Name>Preset_1</tt:Name></tptz:Preset>
<tptz:Preset token="Preset_2"><tt:Name>Preset_2</tt:Name></tptz:Preset>
</tptz:GetPresetsResponse>"#,
    )
}

/// Empty ack for ContinuousMove/Stop/GotoPreset. Pass the response element name,
/// e.g. "tptz:ContinuousMoveResponse".
pub fn ptz_ack(op_response_element: &str) -> String {
    envelope(&format!(r#"<{op_response_element}/>"#))
}

/// Non-2xx body: a SOAP Fault whose Subcode Value contains "NotAuthorized",
/// which is exactly what onvif-rs classifies as an authorization failure.
pub fn fault_not_authorized() -> String {
    envelope(
        r#"<env:Fault><env:Code><env:Value>env:Sender</env:Value>
<env:Subcode><env:Value>ter:NotAuthorized</env:Value></env:Subcode></env:Code>
<env:Reason><env:Text xml:lang="en">Sender not authorized</env:Text></env:Reason>
</env:Fault>"#,
    )
}

pub fn fault_action_not_supported() -> String {
    envelope(
        r#"<env:Fault><env:Code><env:Value>env:Receiver</env:Value>
<env:Subcode><env:Value>ter:ActionNotSupported</env:Value></env:Subcode></env:Code>
<env:Reason><env:Text xml:lang="en">Action not supported</env:Text></env:Reason>
</env:Fault>"#,
    )
}

#[cfg(test)]
#[path = "responses_test.rs"]
mod responses_test;
```

> The templates use real newlines between tags (harmless in XML) — no `\` escapes. The `envelope()` header is on one line so the `<?xml …?>` declaration stays first.

- [ ] **Step 2: Add `mod responses;` to `main.rs`**

Add below `mod auth;`:

```rust
mod responses;
```

- [ ] **Step 3: Write the failing tests (structural)**

Create `examples/dummy-onvif-camera/src/responses_test.rs`:

```rust
use super::*;
use crate::config::DeviceCfg;

fn cfg() -> DeviceCfg {
    DeviceCfg {
        host: "127.0.0.1".into(), port: 8000,
        username: "admin".into(), password: "admin".into(),
        rtsp_url: "rtsp://127.0.0.1:9554/live/test1".into(),
        manufacturer: "lite-nvr".into(), model: "dummy".into(),
        firmware: "0.1".into(), serial: "SN-0001".into(),
    }
}

#[test]
fn capabilities_advertise_media_and_ptz_at_service_url() {
    let x = get_capabilities("http://127.0.0.1:8000/onvif/device_service");
    // both Media and PTZ XAddr point at the service url
    assert_eq!(x.matches("http://127.0.0.1:8000/onvif/device_service").count(), 2);
    assert!(x.contains("GetCapabilitiesResponse"));
}

#[test]
fn stream_uri_contains_rtsp_url() {
    let x = get_stream_uri("rtsp://cam/live");
    assert!(x.contains("<tt:Uri>rtsp://cam/live</tt:Uri>"));
}

#[test]
fn profiles_carry_token_and_resolution() {
    let x = get_profiles();
    assert!(x.contains(r#"token="Profile_1""#));
    assert!(x.contains("<tt:Width>1920</tt:Width>"));
    assert!(x.contains("H264"));
}

#[test]
fn device_information_uses_cfg() {
    let x = device_information(&cfg());
    assert!(x.contains("<tds:Manufacturer>lite-nvr</tds:Manufacturer>"));
    assert!(x.contains("<tds:SerialNumber>SN-0001</tds:SerialNumber>"));
}

#[test]
fn not_authorized_fault_has_subcode() {
    assert!(fault_not_authorized().contains("NotAuthorized"));
}

#[test]
fn presets_have_two_tokens() {
    let x = get_presets();
    assert!(x.contains(r#"token="Preset_1""#));
    assert!(x.contains(r#"token="Preset_2""#));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p dummy-onvif-camera responses`
Expected: PASS (6 tests). If a template's `\` line-continuations leaked into the string, the assertions will fail — switch that template to a clean single-line/real-newline raw string.

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p dummy-onvif-camera
git add examples/dummy-onvif-camera/src
git commit -m "feat(dummy-onvif-camera): SOAP response builders"
```

---

### Task 4: Request parsing + dispatch (`soap.rs`)

**Files:**
- Create: `examples/dummy-onvif-camera/src/soap.rs`
- Create: `examples/dummy-onvif-camera/src/soap_test.rs`
- Modify: `examples/dummy-onvif-camera/src/main.rs` (add `mod soap;`)

**Interfaces:**
- Consumes: `DeviceCfg` (Task 1); `auth::{UsernameToken, verify}` (Task 2); `responses::*` (Task 3).
- Produces: `pub struct Reply { pub status: u16, pub body: String }`; `pub fn detect_op(body: &str) -> Option<&'static str>` returning one of `"GetCapabilities"`,`"GetDeviceInformation"`,`"GetProfiles"`,`"GetStreamUri"`,`"ContinuousMove"`,`"Stop"`,`"GetPresets"`,`"GotoPreset"`; `pub fn extract_token(body: &str) -> Option<auth::UsernameToken>` (parses the WS-Security UsernameToken via quick-xml); `pub fn handle(body: &str, cfg: &DeviceCfg) -> Reply` (dispatch: unknown op → 500 fault; auth fail → 400 NotAuthorized; else 200 + the op response; PTZ ops are logged).

- [ ] **Step 1: Write `soap.rs`**

Create `examples/dummy-onvif-camera/src/soap.rs`:

```rust
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::auth::{self, UsernameToken};
use crate::config::DeviceCfg;
use crate::responses;

pub struct Reply {
    pub status: u16,
    pub body: String,
}

const OPS: &[&str] = &[
    "GetCapabilities",
    "GetDeviceInformation",
    "GetProfiles",
    "GetStreamUri",
    "ContinuousMove",
    "Stop",
    "GetPresets",
    "GotoPreset",
];

/// Detect the ONVIF operation by looking for its request element's local name.
/// GetPresets must be checked before GetProfiles etc.; we test the whole set and
/// return the first that appears as an element local-name in the body.
pub fn detect_op(body: &str) -> Option<&'static str> {
    let mut reader = Reader::from_str(body);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = e.local_name();
                let local = String::from_utf8_lossy(name.as_ref()).to_string();
                if let Some(op) = OPS.iter().find(|op| **op == local) {
                    return Some(op);
                }
            }
            Ok(Event::Eof) => return None,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

/// Extract the WS-Security UsernameToken fields (PasswordDigest mode) if present.
pub fn extract_token(body: &str) -> Option<UsernameToken> {
    let mut reader = Reader::from_str(body);
    let mut buf = Vec::new();
    let (mut username, mut digest, mut nonce, mut created) = (None, None, None, None);
    let mut cur: Option<&'static str> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).to_string();
                cur = match local.as_str() {
                    "Username" => Some("Username"),
                    "Password" => Some("Password"),
                    "Nonce" => Some("Nonce"),
                    "Created" => Some("Created"),
                    _ => None,
                };
            }
            Ok(Event::Text(t)) => {
                if let Some(field) = cur {
                    let val = t.unescape().unwrap_or_default().to_string();
                    match field {
                        "Username" => username = Some(val),
                        "Password" => digest = Some(val),
                        "Nonce" => nonce = Some(val),
                        "Created" => created = Some(val),
                        _ => {}
                    }
                }
            }
            Ok(Event::End(_)) => cur = None,
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
    Some(UsernameToken {
        username: username?,
        password_digest: digest?,
        nonce: nonce?,
        created: created?,
    })
}

/// Authenticate + dispatch. Every operation requires a valid UsernameToken.
pub fn handle(body: &str, cfg: &DeviceCfg) -> Reply {
    let Some(op) = detect_op(body) else {
        return Reply { status: 500, body: responses::fault_action_not_supported() };
    };

    let authed = extract_token(body)
        .map(|t| auth::verify(&t, &cfg.username, &cfg.password))
        .unwrap_or(false);
    if !authed {
        log::warn!("onvif {op}: rejected (bad/missing UsernameToken)");
        return Reply { status: 400, body: responses::fault_not_authorized() };
    }

    let body = match op {
        "GetCapabilities" => responses::get_capabilities(&cfg.service_url()),
        "GetDeviceInformation" => responses::device_information(cfg),
        "GetProfiles" => responses::get_profiles(),
        "GetStreamUri" => responses::get_stream_uri(&cfg.rtsp_url),
        "GetPresets" => responses::get_presets(),
        "ContinuousMove" => {
            log::info!("onvif PTZ: ContinuousMove");
            responses::ptz_ack("tptz:ContinuousMoveResponse")
        }
        "Stop" => {
            log::info!("onvif PTZ: Stop");
            responses::ptz_ack("tptz:StopResponse")
        }
        "GotoPreset" => {
            log::info!("onvif PTZ: GotoPreset");
            responses::ptz_ack("tptz:GotoPresetResponse")
        }
        _ => unreachable!("op is from OPS"),
    };
    Reply { status: 200, body }
}

#[cfg(test)]
#[path = "soap_test.rs"]
mod soap_test;
```

- [ ] **Step 2: Add `mod soap;` to `main.rs`**

Add below `mod responses;`:

```rust
mod soap;
```

- [ ] **Step 3: Write the failing tests**

Create `examples/dummy-onvif-camera/src/soap_test.rs`:

```rust
use super::*;
use crate::auth::password_digest;
use crate::config::DeviceCfg;

fn cfg() -> DeviceCfg {
    DeviceCfg {
        host: "127.0.0.1".into(), port: 8000,
        username: "admin".into(), password: "secret".into(),
        rtsp_url: "rtsp://127.0.0.1:9554/live/test1".into(),
        manufacturer: "lite-nvr".into(), model: "dummy".into(),
        firmware: "0.1".into(), serial: "SN-0001".into(),
    }
}

/// Build a SOAP request with a WS-Security header for `op`.
fn req(op: &str, user: &str, pass: &str) -> String {
    let nonce = "MTIzNDU2Nzg5MDEyMzQ1Ng==";
    let created = "2026-07-20T00:00:00Z";
    let digest = password_digest(nonce, created, pass);
    format!(
        r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"
 xmlns:w="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd"
 xmlns:u="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd">
<s:Header><w:Security><w:UsernameToken>
<w:Username>{user}</w:Username>
<w:Password Type="...#PasswordDigest">{digest}</w:Password>
<w:Nonce>{nonce}</w:Nonce><u:Created>{created}</u:Created>
</w:UsernameToken></w:Security></s:Header>
<s:Body><trt:{op} xmlns:trt="http://www.onvif.org/ver10/media/wsdl"/></s:Body></s:Envelope>"#
    )
}

#[test]
fn detects_operations() {
    assert_eq!(detect_op(&req("GetProfiles", "a", "b")), Some("GetProfiles"));
    assert_eq!(detect_op(&req("GetStreamUri", "a", "b")), Some("GetStreamUri"));
    assert_eq!(detect_op("<x/>"), None);
}

#[test]
fn valid_auth_returns_200_and_op_response() {
    let r = handle(&req("GetStreamUri", "admin", "secret"), &cfg());
    assert_eq!(r.status, 200);
    assert!(r.body.contains("rtsp://127.0.0.1:9554/live/test1"));
}

#[test]
fn wrong_password_returns_400_not_authorized() {
    let r = handle(&req("GetProfiles", "admin", "WRONG"), &cfg());
    assert_eq!(r.status, 400);
    assert!(r.body.contains("NotAuthorized"));
}

#[test]
fn missing_security_returns_400() {
    let no_hdr = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body>
<trt:GetProfiles xmlns:trt="http://www.onvif.org/ver10/media/wsdl"/></s:Body></s:Envelope>"#;
    let r = handle(no_hdr, &cfg());
    assert_eq!(r.status, 400);
    assert!(r.body.contains("NotAuthorized"));
}

#[test]
fn unknown_op_returns_fault() {
    let unknown = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body>
<trt:GetSomethingElse xmlns:trt="x"/></s:Body></s:Envelope>"#;
    let r = handle(unknown, &cfg());
    assert_eq!(r.status, 500);
    assert!(r.body.contains("ActionNotSupported"));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p dummy-onvif-camera soap`
Expected: PASS (5 soap tests).

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p dummy-onvif-camera
git add examples/dummy-onvif-camera/src
git commit -m "feat(dummy-onvif-camera): SOAP request parsing + auth dispatch"
```

---

### Task 5: WS-Discovery ProbeMatches (`discovery.rs`)

**Files:**
- Create: `examples/dummy-onvif-camera/src/discovery.rs`
- Create: `examples/dummy-onvif-camera/src/discovery_test.rs`
- Modify: `examples/dummy-onvif-camera/src/main.rs` (add `mod discovery;`)

**Interfaces:**
- Consumes: `DeviceCfg` (Task 1).
- Produces: `pub fn extract_message_id(probe: &str) -> Option<String>` (the probe's `wsa:MessageID`, for `RelatesTo`); `pub fn probe_matches_xml(msg_id: &str, relates_to: &str, xaddr: &str, name: &str, hardware: &str) -> String` (the ProbeMatches SOAP-over-UDP envelope); `pub async fn run(cfg: DeviceCfg, cancel: tokio_util::sync::CancellationToken)` — the UDP multicast responder (integration; validated in Task 7). NOTE: `tokio-util` isn't a dep yet; use a `tokio::sync::watch`/`Notify` or an owned `std::sync::atomic::AtomicBool` for cancel instead to avoid adding a dep — simplest: `run(cfg)` loops forever and is aborted by dropping its `JoinHandle` on shutdown.

- [ ] **Step 1: Write `discovery.rs`**

Create `examples/dummy-onvif-camera/src/discovery.rs`:

```rust
use std::net::{Ipv4Addr, SocketAddr};

use quick_xml::Reader;
use quick_xml::events::Event;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use uuid::Uuid;

use crate::config::DeviceCfg;

const WS_DISCOVERY_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const WS_DISCOVERY_PORT: u16 = 3702;

/// Pull `wsa:MessageID` out of a Probe so we can echo it as RelatesTo.
pub fn extract_message_id(probe: &str) -> Option<String> {
    let mut reader = Reader::from_str(probe);
    let mut buf = Vec::new();
    let mut in_id = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                in_id = e.local_name().as_ref() == b"MessageID";
            }
            Ok(Event::Text(t)) if in_id => {
                return Some(t.unescape().unwrap_or_default().to_string());
            }
            Ok(Event::End(_)) => in_id = false,
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

/// A ProbeMatches reply. Scopes carry the device name + hardware, which the
/// onvif-rs client surfaces as `Device.name` / `Device.hardware`.
pub fn probe_matches_xml(
    msg_id: &str,
    relates_to: &str,
    xaddr: &str,
    name: &str,
    hardware: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope"
 xmlns:w="http://schemas.xmlsoap.org/ws/2004/08/addressing"
 xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery"
 xmlns:dn="http://www.onvif.org/ver10/network/wsdl">
<e:Header>
<w:MessageID>urn:uuid:{msg_id}</w:MessageID>
<w:RelatesTo>{relates_to}</w:RelatesTo>
<w:To>http://schemas.xmlsoap.org/ws/2004/08/addressing/role/anonymous</w:To>
<w:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/ProbeMatches</w:Action>
</e:Header>
<e:Body><d:ProbeMatches><d:ProbeMatch>
<w:EndpointReference><w:Address>urn:uuid:{msg_id}</w:Address></w:EndpointReference>
<d:Types>dn:NetworkVideoTransmitter</d:Types>
<d:Scopes>onvif://www.onvif.org/type/video_encoder onvif://www.onvif.org/name/{name} onvif://www.onvif.org/hardware/{hardware} onvif://www.onvif.org/location/dummy</d:Scopes>
<d:XAddrs>{xaddr}</d:XAddrs>
<d:MetadataVersion>1</d:MetadataVersion>
</d:ProbeMatch></d:ProbeMatches></e:Body></e:Envelope>"#
    )
}

/// Bind 3702, join the multicast group, and answer every Probe with a unicast
/// ProbeMatches. Runs until the task is aborted.
pub async fn run(cfg: DeviceCfg) -> anyhow::Result<()> {
    // Build a reuse-addr UDP socket bound to 0.0.0.0:3702 and join the group.
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    let bind: SocketAddr = format!("0.0.0.0:{WS_DISCOVERY_PORT}").parse()?;
    sock.bind(&bind.into())?;
    sock.join_multicast_v4(&WS_DISCOVERY_ADDR, &Ipv4Addr::UNSPECIFIED)?;
    sock.set_nonblocking(true)?;
    let udp = UdpSocket::from_std(sock.into())?;

    let xaddr = cfg.service_url();
    let mut buf = vec![0u8; 64 * 1024];
    log::info!("ws-discovery: listening on 239.255.255.250:{WS_DISCOVERY_PORT}");
    loop {
        let (n, from) = match udp.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("ws-discovery recv: {e}");
                continue;
            }
        };
        let probe = String::from_utf8_lossy(&buf[..n]);
        if !probe.contains("Probe") {
            continue;
        }
        let relates_to = extract_message_id(&probe).unwrap_or_default();
        let reply = probe_matches_xml(
            &Uuid::new_v4().to_string(),
            &relates_to,
            &xaddr,
            &cfg.model,
            &cfg.manufacturer,
        );
        if let Err(e) = udp.send_to(reply.as_bytes(), from).await {
            log::warn!("ws-discovery send: {e}");
        } else {
            log::info!("ws-discovery: answered probe from {from}");
        }
    }
}

#[cfg(test)]
#[path = "discovery_test.rs"]
mod discovery_test;
```

> `Uuid::new_v4()` needs the `v4` feature (declared in Task 1's Cargo.toml). `socket2` is the standard way to set `SO_REUSEADDR` before bind so re-runs don't fail with "address in use".

- [ ] **Step 2: Add `mod discovery;` to `main.rs`**

Add below `mod soap;`:

```rust
mod discovery;
```

- [ ] **Step 3: Write the failing tests**

Create `examples/dummy-onvif-camera/src/discovery_test.rs`:

```rust
use super::*;

#[test]
fn extracts_probe_message_id() {
    let probe = r#"<e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope"
 xmlns:w="http://schemas.xmlsoap.org/ws/2004/08/addressing"><e:Header>
<w:MessageID>urn:uuid:abc-123</w:MessageID></e:Header><e:Body/></e:Envelope>"#;
    assert_eq!(extract_message_id(probe).as_deref(), Some("urn:uuid:abc-123"));
}

#[test]
fn probe_matches_carries_xaddr_and_scopes() {
    let x = probe_matches_xml(
        "id-1",
        "urn:uuid:abc-123",
        "http://127.0.0.1:8000/onvif/device_service",
        "dummy-model",
        "lite-nvr",
    );
    assert!(x.contains("<d:XAddrs>http://127.0.0.1:8000/onvif/device_service</d:XAddrs>"));
    assert!(x.contains("onvif://www.onvif.org/name/dummy-model"));
    assert!(x.contains("onvif://www.onvif.org/hardware/lite-nvr"));
    assert!(x.contains("<w:RelatesTo>urn:uuid:abc-123</w:RelatesTo>"));
    assert!(x.contains("ProbeMatches"));
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p dummy-onvif-camera discovery`
Expected: PASS (2 tests). (`run` has no unit test — validated in Task 7.)

- [ ] **Step 5: Format & commit**

```bash
cargo fmt -p dummy-onvif-camera
git add examples/dummy-onvif-camera/src
git commit -m "feat(dummy-onvif-camera): WS-Discovery ProbeMatches responder"
```

---

### Task 6: Wire up `main.rs` (HTTP server + discovery + optional RTSP)

**Files:**
- Modify: `examples/dummy-onvif-camera/src/main.rs`

**Interfaces:**
- Consumes: `config`, `soap::handle`, `discovery::run` (Tasks 1/4/5); `axum`, `tokio`, `which`.

> Integration task. Gate: `cargo build -p dummy-onvif-camera` compiles cleanly. Runtime is validated in Task 7.

- [ ] **Step 1: Replace `main.rs` body with the full wiring**

Rewrite `examples/dummy-onvif-camera/src/main.rs` (keep the `mod` lines at the top; they were added across Tasks 2–5):

```rust
mod auth;
mod config;
mod discovery;
mod responses;
mod soap;

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use clap::Parser;

use crate::config::DeviceCfg;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let (cfg, opts) = config::Args::parse().into_cfg();
    let cfg = Arc::new(cfg);

    // Optional: spawn dummy-rtsp-camera so the RTSP url actually serves.
    let mut _rtsp_child = None;
    if opts.launch_rtsp {
        match which::which("cargo") {
            Ok(cargo) => {
                log::info!("launching dummy-rtsp-camera …");
                _rtsp_child = Some(
                    std::process::Command::new(cargo)
                        .args(["run", "-q", "-p", "dummy-rtsp-camera"])
                        .spawn()?,
                );
            }
            Err(_) => log::warn!("--launch-rtsp: cargo not found; run dummy-rtsp-camera yourself"),
        }
    } else {
        log::info!("GetStreamUri will return {} — run dummy-rtsp-camera to serve it", cfg.rtsp_url);
    }

    // WS-Discovery responder.
    if opts.discovery {
        let dcfg = (*cfg).clone();
        tokio::spawn(async move {
            if let Err(e) = discovery::run(dcfg).await {
                log::error!("ws-discovery stopped: {e}");
            }
        });
    }

    // SOAP HTTP server.
    let app = Router::new()
        .route("/onvif/device_service", post(soap_handler))
        .with_state(cfg.clone());
    let addr = format!("0.0.0.0:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    log::info!("dummy-onvif-camera: SOAP at {} (user {})", cfg.service_url(), cfg.username);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn soap_handler(State(cfg): State<Arc<DeviceCfg>>, body: Bytes) -> Response {
    let text = String::from_utf8_lossy(&body);
    let reply = soap::handle(&text, &cfg);
    Response::builder()
        .status(StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header("Content-Type", "application/soap+xml; charset=utf-8")
        .body(reply.body.into())
        .expect("valid response")
}
```

- [ ] **Step 2: Verify it compiles and existing tests pass**

Run: `cargo test -p dummy-onvif-camera`
Expected: builds; all 19 unit tests from Tasks 1–5 pass (2 config + 4 auth + 6 responses + 5 soap + 2 discovery).

- [ ] **Step 3: Smoke-check it starts and rejects unauthenticated calls**

Run (background), then curl:

```bash
cargo run -q -p dummy-onvif-camera -- --port 8000 &
sleep 2
# no auth header -> 400 with NotAuthorized fault
curl -s -o /dev/null -w '%{http_code}\n' -X POST http://127.0.0.1:8000/onvif/device_service \
  -H 'Content-Type: application/soap+xml' \
  --data '<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body><trt:GetProfiles xmlns:trt="http://www.onvif.org/ver10/media/wsdl"/></s:Body></s:Envelope>'
kill %1
```
Expected: prints `400`.

- [ ] **Step 4: Format & commit**

```bash
cargo fmt -p dummy-onvif-camera
git add examples/dummy-onvif-camera/src/main.rs
git commit -m "feat(dummy-onvif-camera): HTTP SOAP server + discovery wiring"
```

---

### Task 7: End-to-end validation against `nvr-onvif` + README

**Files:**
- Create: `examples/dummy-onvif-camera/README.md`
- Modify (only if the E2E run requires it): `examples/dummy-onvif-camera/src/responses.rs` and/or `src/discovery.rs` (XML adaptation).

**Interfaces:**
- Consumes: the whole example + `crates/nvr-onvif`'s ignored `tests/live.rs`.

> This is the definitive gate for XML parse-compatibility. Run the real client against the dummy and adapt the response templates until it passes. Do NOT change the pure function signatures — only XML string contents.

- [ ] **Step 1: Run the dummy and point the `nvr-onvif` live test at it**

```bash
# terminal 1
cargo run -q -p dummy-onvif-camera -- --host 127.0.0.1 --port 8000 --username admin --password admin
# terminal 2
ONVIF_TEST_HOST=127.0.0.1 ONVIF_TEST_PORT=8000 ONVIF_TEST_USER=admin ONVIF_TEST_PASS=admin \
  cargo test -p nvr-onvif --test live -- --ignored --nocapture
```
Expected: `connect_profiles_stream_uri_ptz` prints the device info + an `rtsp://…` URI and passes.

- [ ] **Step 2: Adapt the XML if the client fails to parse**

If the test fails, the error names the operation whose response didn't deserialize. For that operation, compare the dummy's template element/namespace against what the `schema` crate expects: read the matching type in the pinned dep, e.g.
`find ~/.cargo/git/checkouts/onvif-rs-* -path '*schema/*'` then the `media`/`devicemgmt`/`ptz` modules and the `onvif_xsd`/`common` types, and align the element local-names + namespaces in `responses.rs` (or `probe_matches_xml` in `discovery.rs`). Re-run Step 1 until green. Keep the `responses::*` / `discovery::*` signatures unchanged; only edit the XML strings. Re-run `cargo test -p dummy-onvif-camera` afterward to confirm the structural unit tests still pass (adjust their asserted substrings if you renamed an element).

- [ ] **Step 3: Verify discovery finds the dummy (optional but recommended)**

With the dummy running (discovery on), write a 10-line throwaway `main` or reuse a quick check that calls `nvr_onvif::discover(Duration::from_secs(3))` and prints the results; confirm the dummy's `XAddrs`/name appear. (This path is best-effort — if multicast is restricted in the sandbox, note it in the README and rely on the direct `probe`/live-test path.)

- [ ] **Step 4: Write the README**

Create `examples/dummy-onvif-camera/README.md` documenting: what it simulates, the CLI flags, the two-terminal quickstart (dummy + `dummy-rtsp-camera`), how to point the `nvr-onvif` live test at it (the Step 1 commands), and the full `nvr` end-to-end (start both dummies, add an `input_type=onvif` device with host `127.0.0.1` port `8000` admin/admin → nvr discovers the RTSP URI and records; dashboard PTZ buttons log moves; a wrong password on 「探测」 shows "authentication rejected"). Note the out-of-scope items (no events/imaging/Media2, single profile, PTZ logged only).

- [ ] **Step 5: Commit**

```bash
cargo fmt -p dummy-onvif-camera
git add examples/dummy-onvif-camera
git commit -m "test(dummy-onvif-camera): end-to-end validation against nvr-onvif + README"
```

---

## Notes for the implementer

- **The XML is the risk.** Tasks 3/5 write best-effort ONVIF XML; Task 7 proves it against the real client and is where any namespace/element mismatch is fixed. Budget the most time there. The auth-fault shape is already pinned (`NotAuthorized` subcode → HTTP 400), so the Auth path should work first try.
- **Response templates** use real newlines between tags (valid XML) — transcribe them as-is. If you reformat, keep the `<?xml …?>` declaration first (it must be at byte 0 of the body) and don't introduce `\` escapes.
- **quick-xml 0.37:** the parsing code reads events without `trim_text` config (the values we extract are compact in real requests). If a real request wraps a `Nonce`/`Created` text node in whitespace and the digest check fails, `.trim()` only those two values — do NOT trim globally.
- **Multicast in the sandbox:** WS-Discovery may not round-trip if the environment restricts multicast. The direct `probe`/`GetStreamUri`/PTZ path (Task 7 Step 1) does not need multicast and is the primary validation; discovery (Step 3) is secondary.
- **No `crates/*` or `nvr` edits** — this is a standalone example. The only cross-crate touch is Task 7 *running* `nvr-onvif`'s existing ignored test against the dummy.
