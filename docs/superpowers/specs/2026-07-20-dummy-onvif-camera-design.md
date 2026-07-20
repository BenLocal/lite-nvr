# Dummy ONVIF camera example

Date: 2026-07-20
Status: approved (hand-rolled minimal ONVIF SOAP + WS-Discovery in the example;
RTSP plane reuses dummy-rtsp-camera/oddity; WS-Security UsernameToken validated
against configured credentials so the Auth-reject path is exercised)

## Problem

`nvr-onvif` (discovery, probe, stream-URI, PTZ) and its `nvr` integration have
no local device to test against — the ignored `tests/live.rs` needs a real
camera. We want a simulated ONVIF camera under `examples/` that answers exactly
the operations `nvr-onvif` calls, so discovery → probe → RTSP ingestion → PTZ
can be verified end to end on one machine.

## Why hand-rolled

There is **no ONVIF *server* library in Rust** (lumeohq/onvif-rs is client-only)
and no external ONVIF simulator is available in this environment (apt ships only
ONVIF *client* libs; `onvif_srvd` needs a gsoap+C++ build; Python simulators need
pip and vary in quality). The surface `nvr-onvif` actually exercises is tiny and
fixed — eight SOAP operations (mostly static XML) plus one UDP WS-Discovery
reply — so hand-serving it inside the example is self-contained and lower-risk
than any external dependency. The actual video is **not** re-implemented: the
example reuses the existing `dummy-rtsp-camera` (oddity-rtsp-server) for RTSP.

## Exactly what the client calls (the whole contract)

From `crates/nvr-onvif/src/{camera,discovery}.rs`:

- **WS-Discovery**: multicast `Probe`; client reads each match's `XAddrs`
  (→ endpoints), scope-derived `name`, and `hardware`.
- **Device service** (`http://host:port/onvif/device_service`):
  `GetCapabilities` (client reads `capabilities.media.x_addr` and, if present,
  `capabilities.ptz.x_addr`), `GetDeviceInformation`
  (manufacturer/model/firmware/serial).
- **Media service**: `GetProfiles` (token, name, video-encoder resolution +
  encoding), `GetStreamUri` (→ `media_uri.uri`, the RTSP URL).
- **PTZ service**: `ContinuousMove`, `Stop`, `GetPresets`, `GotoPreset`.

The dummy advertises **media and PTZ at the same URL** as the device service
(legal per ONVIF and simplest), so one HTTP endpoint serves everything.

## Architecture

New binary crate `examples/dummy-onvif-camera` (added to root `Cargo.toml`
members; same shape as `examples/dummy-rtsp-camera`). Three concurrent parts,
all under one Tokio runtime:

1. **SOAP HTTP server** (axum) on `--port` — one handler at
   `/onvif/device_service` that authenticates, dispatches by operation, and
   returns the canned response.
2. **WS-Discovery responder** (UDP) — joins `239.255.255.250:3702`, replies to
   each `Probe` with a unicast `ProbeMatches`.
3. **RTSP plane** — reuses `dummy-rtsp-camera`. Default: `GetStreamUri` returns
   `--rtsp-url` and the example prints a reminder to run `dummy-rtsp-camera`;
   with `--launch-rtsp` it spawns `dummy-rtsp-camera` as a child process.

### Files

- `src/main.rs` — CLI (clap), logging, spawn the HTTP + UDP tasks, optional
  `dummy-rtsp-camera` child, print the advertised service URL, run until Ctrl-C.
- `src/soap.rs` — `handle(body: &str, cfg: &DeviceCfg) -> SoapReply`: detect the
  operation (by top-level body element name), verify auth, build the response;
  returns either a 200 SOAP response or a fault/401. Pure over `&str` + config
  → unit-testable.
- `src/responses.rs` — the response XML builders (`get_capabilities`,
  `device_information`, `get_profiles`, `get_stream_uri`, `get_presets`,
  `ptz_ack`, `fault`) with substitution. Pure functions → unit-testable.
- `src/discovery.rs` — the UDP multicast `Probe` → `ProbeMatches` responder, and
  a pure `probe_matches_xml(uuid, xaddr, name, hardware) -> String` builder.
- `src/auth.rs` — WS-Security UsernameToken `PasswordDigest` verification:
  `verify(security_header, password) -> bool` where the digest is
  `Base64(SHA1(nonce_bytes ++ created ++ password))`. Pure → unit-testable.

### CLI

```
--host <IP>          advertised media/service IP (default 127.0.0.1)
--port <PORT>        ONVIF HTTP port (default 8000)
--username <U>       required WS-Security username (default admin)
--password <P>       required WS-Security password (default admin)
--rtsp-url <URL>     URL returned by GetStreamUri
                     (default rtsp://127.0.0.1:9554/live/test1 — dummy-rtsp-camera)
--launch-rtsp        also spawn dummy-rtsp-camera as a child (default off)
--no-discovery       disable the WS-Discovery responder (it runs by default)
--manufacturer/--model/--firmware/--serial   DeviceInformation fields (defaults)
```

## Data flow

1. `nvr-onvif::discover()` multicasts a `Probe`; the responder replies
   `ProbeMatches` with `XAddrs = http://{host}:{port}/onvif/device_service`,
   scopes carrying `name`/`hardware`. Client surfaces it as a `Discovered`.
2. `probe`/`connect` → `GetCapabilities` returns media + ptz `XAddr` both equal
   to the device service URL. `GetDeviceInformation` returns the configured
   fields.
3. `GetProfiles` returns one profile (`Profile_1`, `H264`, `1920x1080`);
   `GetStreamUri` returns `--rtsp-url`. `nvr-onvif` then injects credentials and
   pulls that RTSP into ZLM via the existing pipeline.
4. PTZ: `ContinuousMove`/`Stop`/`GotoPreset` are logged (a real camera would move
   motors); `GetPresets` returns two canned presets (`Preset_1`/`Preset_2`).

## Authentication

ONVIF mandates WS-Security UsernameToken. The SOAP handler, for every operation:

- extracts the `<Security>` header's `Username`, `Password` (Type
  `#PasswordDigest`), `Nonce`, `Created`;
- recomputes `Base64(SHA1(base64_decode(nonce) ++ created ++ password))` with the
  configured `--password` and compares to the supplied digest, and checks the
  username matches `--username`;
- on missing/mismatch, returns the response that `onvif-rs` classifies as
  `transport::Error::Authorization` (so the client yields `OnvifError::Auth`).
  The exact trigger — HTTP `401` vs a `ter:NotAuthorized` SOAP fault — is
  confirmed by reading the pinned `onvif/src/soap/client.rs` during
  implementation; whichever it keys `Authorization` on is what the dummy returns.

`nvr-onvif` uses `AuthType::Any`, which includes the UsernameToken header on
authenticated calls; if `Any` first probes unauthenticated, the dummy's
Authorization response prompts the retry-with-credentials, exactly as a real
device would.

## Error handling

- Unknown/unsupported operation → a SOAP `env:Receiver`/`ter:ActionNotSupported`
  fault body with HTTP 500 (standard SOAP-fault status). This is a defensive
  path — `nvr-onvif` only calls the eight supported operations.
- Malformed/no SOAP body → HTTP 400.
- Auth missing/wrong → the Authorization-triggering response (above).
- The RTSP plane is independent: if `dummy-rtsp-camera` isn't running and
  `--launch-rtsp` is off, ONVIF still answers; only the actual pull fails, which
  is the operator's cue to start the RTSP source.

## Testing

- **Unit (pure, no network):**
  - `responses.rs`: each builder substitutes host/port/rtsp-url/presets into the
    expected element/namespace shape (assert the RTSP URL appears in
    `GetStreamUri`, the profile token/resolution in `GetProfiles`, the two
    service `XAddr`s in `GetCapabilities`).
  - `auth.rs`: `verify` accepts a token digest computed from the right password
    and rejects a wrong-password digest and a missing token.
  - `discovery.rs`: `probe_matches_xml` contains the `XAddr` and scopes.
  - `soap.rs`: `handle` dispatches each operation name to the matching builder
    and returns the fault for an unknown op.
- **End-to-end payoff (manual / documented):**
  - Run the dummy, then
    `ONVIF_TEST_HOST=127.0.0.1 ONVIF_TEST_PORT=8000 ONVIF_TEST_USER=admin
    ONVIF_TEST_PASS=admin cargo test -p nvr-onvif --test live -- --ignored` —
    the previously camera-only integration test now passes against the dummy.
  - Full `nvr` run: start `dummy-rtsp-camera` + the dummy ONVIF, add an `onvif`
    device (host `127.0.0.1`, port `8000`, admin/admin) → nvr resolves the RTSP
    URI and records; the dashboard PTZ buttons log moves on the dummy.
  - A wrong password on the add-device 「探测」 shows the "authentication
    rejected" message (the Auth path).

## Out of scope

ONVIF events / imaging / Media2, multiple media profiles, real PTZ position
tracking, TLS/HTTPS, RTSP re-implementation (reuse dummy-rtsp-camera).
