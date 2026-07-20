# dummy-onvif-camera — a simulated ONVIF camera

A runnable "dummy ONVIF camera" that behaves like a real IP camera's ONVIF
service toward `nvr-onvif` (and the NVR). It hand-serves the ONVIF SOAP calls the
client actually makes, answers WS-Discovery probes, and reuses
`dummy-rtsp-camera` for the real RTSP media plane.

## What it simulates

- **WS-Security auth (PasswordDigest).** Every operation requires a valid
  UsernameToken (`Base64(SHA1(nonce + created + password))`). A missing or wrong
  token gets a SOAP 1.2 Fault with subcode `ter:NotAuthorized` over HTTP 400 —
  exactly what onvif-rs classifies as an authorization failure, so the NVR's
  probe surfaces "**authentication rejected**".
- **Device management:** `GetCapabilities` (advertises Media + PTZ service
  addresses, both pointing at this same endpoint) and `GetDeviceInformation`
  (manufacturer / model / firmware / serial).
- **Media:** `GetProfiles` (a single `Profile_1`, H264 1920×1080) and
  `GetStreamUri` (returns the configured `--rtsp-url`).
- **PTZ:** `ContinuousMove`, `Stop`, `GetPresets`, `GotoPreset`. Moves are
  **logged only** (a real camera would drive its motors); the response is an
  empty ACK.
- **WS-Discovery:** joins `239.255.255.250:3702` and answers every `Probe` with a
  unicast `ProbeMatches` carrying this device's `XAddrs`, name, and hardware in
  the scopes.

The SOAP endpoint is the ONVIF well-known entry point:
`http://<host>:<port>/onvif/device_service`.

## CLI flags

```bash
cargo run -q -p dummy-onvif-camera -- --host 127.0.0.1 --port 8000 \
  --username admin --password admin
```

| Flag | Meaning | Default |
|------|---------|---------|
| `--host` | Advertised media/service IP (used in the service + stream URLs) | `127.0.0.1` |
| `--port` | ONVIF SOAP HTTP port | `8000` |
| `--username` | Required WS-Security username | `admin` |
| `--password` | Required WS-Security password | `admin` |
| `--rtsp-url` | URL returned by `GetStreamUri` | `rtsp://127.0.0.1:9554/live/test1` |
| `--launch-rtsp` | Also spawn `dummy-rtsp-camera` as a child process | `false` |
| `--no-discovery` | Disable the WS-Discovery responder (on by default) | `false` |
| `--manufacturer` / `--model` / `--firmware` / `--serial` | `GetDeviceInformation` fields | `lite-nvr` / `dummy-onvif-camera` / `0.1` / `SN-0001` |

Set `RUST_LOG=info` (or `debug`) for request logs (auth rejects, PTZ moves,
answered probes).

## Quickstart (two terminals)

The default `--rtsp-url` (`rtsp://127.0.0.1:9554/live/test1`) matches
`dummy-rtsp-camera`'s defaults, so the URI the ONVIF camera hands out actually
serves video.

```bash
# terminal 1 — the RTSP media plane
cargo run -q -p dummy-rtsp-camera -- --port 9554 --path /live/test1

# terminal 2 — the ONVIF service
cargo run -q -p dummy-onvif-camera -- --host 127.0.0.1 --port 8000 \
  --username admin --password admin
```

Or fold both into one process with `--launch-rtsp`:

```bash
cargo run -q -p dummy-onvif-camera -- --host 127.0.0.1 --port 8000 \
  --username admin --password admin --launch-rtsp
```

## Point the `nvr-onvif` live test at it

With the dummy running, the real client's ignored end-to-end test (connect →
device info → profiles → stream URI → best-effort PTZ) passes against it. It is
pure Rust — no `LD_LIBRARY_PATH` needed:

```bash
ONVIF_TEST_HOST=127.0.0.1 ONVIF_TEST_PORT=8000 \
ONVIF_TEST_USER=admin ONVIF_TEST_PASS=admin \
  cargo test -p nvr-onvif --test live -- --ignored --nocapture
```

Expected:

```
device: lite-nvr dummy-onvif-camera fw 0.1
test connect_profiles_stream_uri_ptz ... ok
```

(The onvif-rs client uses `AuthType::Any`: it tries each op once with no auth,
takes the `NotAuthorized` fault, then retries with the WS-Security digest — so
the dummy's log shows one `rejected` line per op followed by the successful
call. That is normal.)

## Full end-to-end against the NVR

Start both dummies (as in the Quickstart), then start the NVR (see the repo
README for `FFMPEG_DIR` / `ZLM_DIR` / `LD_LIBRARY_PATH`):

```bash
cargo run --package nvr
```

Add an `input_type = "onvif"` device whose `input_value` is a serialized
`OnvifConfig` pointing at the dummy (the API needs a session token — log in as
`admin`/`admin` and pass `?token=…`):

```bash
curl -s localhost:8080/api/device/add -H 'content-type: application/json' -d '{
  "id": "cam-onvif",
  "name": "Dummy ONVIF",
  "input_type": "onvif",
  "input_value": "{\"host\":\"127.0.0.1\",\"port\":8000,\"username\":\"admin\",\"password\":\"admin\",\"profile_token\":null}"
}'
```

Then:

1. **Probe from the dashboard** (Device list → 「探测」, host `127.0.0.1` port
   `8000` admin/admin). The NVR calls `GetDeviceInformation` + `GetProfiles` and
   shows 「探测成功」 with `lite-nvr dummy-onvif-camera` and the `Profile_1` video
   configuration.
   - **Wrong password** → the probe fails with 「探测失败」 and the underlying
     error is **"authentication rejected"** (the `NotAuthorized` fault path).
2. **Record.** The ONVIF ingest worker resolves the RTSP URI from the camera's
   media service (`GetStreamUri`), folds in the credentials, and drives the same
   RTSP → ZLM pipe the livestream path uses. The NVR logs
   `onvif cam-onvif: resolved rtsp uri`. The URI self-heals on reconnect because
   it is re-resolved from ONVIF each time.
3. **PTZ.** The dashboard PTZ buttons hit `/api/onvif/ptz`; the dummy logs
   `onvif PTZ: ContinuousMove` / `Stop` / `GotoPreset` (a real camera would move
   its motors).
4. **Discovery** (optional). The NVR's `POST /api/onvif/discover` returns the
   dummy with its `XAddrs`, name (`dummy-onvif-camera`), and hardware
   (`lite-nvr`) — subject to the multicast caveat below.

## WS-Discovery / multicast caveat

Discovery was verified to round-trip in this environment:
`nvr_onvif::discover(3s)` found the dummy (name `dummy-onvif-camera`, hardware
`lite-nvr`, addr `127.0.0.1:8000`) and the dummy logged `answered probe`.
However, WS-Discovery relies on IPv4 multicast on `239.255.255.250:3702`, which
some restricted sandboxes / containers do not route. If discovery returns
nothing, fall back to the **direct** path: point the `nvr-onvif` live test (or the
NVR's 「探测」 with an explicit host/port) at the dummy — that path uses plain
unicast HTTP and does not need multicast. It is the primary validation.

## Out of scope

- **No events, imaging, or Media2** — only devicemgmt / media (ver10) / ptz
  (ver20) operations the `nvr-onvif` client calls.
- **Single media profile** (`Profile_1`, H264 1920×1080).
- **PTZ is logged only** — moves and preset gotos are ACKed but not simulated.
- Audio is out of scope; the RTSP plane is whatever `dummy-rtsp-camera` serves.
