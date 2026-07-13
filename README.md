# lite-nvr

A lightweight Network Video Recorder (NVR) built with Rust. It ingests video
from RTSP/RTMP, local files, screen capture, test patterns, and V4L2 devices,
transcodes via FFmpeg, and distributes through
[ZLMediaKit](https://github.com/ZLMediaKit/ZLMediaKit) for RTSP/RTMP/HLS
streaming. It ships with a REST API server and a Vue 3 web dashboard.

## Features

- 🎥 Multiple input sources: RTSP/RTMP network streams, local files, screen capture, V4L2 devices, and test patterns
- 🔄 Real-time FFmpeg transcoding (H.264 / libx264)
- 📡 Stream distribution through ZLMediaKit (RTSP / RTMP / HLS)
- 🗂️ HLS-based recording with per-segment playback
- 🌐 RESTful API and a Vue 3 web dashboard
- 💾 SQLite persistence (WAL mode)
- ⚡ Async pipeline architecture (Tokio + async channels)

## Architecture

```
┌──────────┐    ┌───────────┐    ┌─────────┐    ┌─────────────┐
│  Input   │───▶│  Decoder  │───▶│ Encoder │───▶│   Output    │
│ (source) │    │ (ffmpeg)  │    │(libx264)│    │(ZLM/file/…) │
└──────────┘    └───────────┘    └─────────┘    └─────────────┘
```

### Workspace crates

| Crate             | Description                                                                |
| ----------------- | -------------------------------------------------------------------------- |
| **nvr**           | Main app — REST API (Axum), pipeline lifecycle, ZLMediaKit integration     |
| **ffmpeg-bus**    | Media engine — input demux, decoder, encoder, muxer wired via async channels |
| **nvr-db**        | Database layer — SQLite/Turso with embedded SQL migrations and a KV store  |
| **nvr-dashboard** | Web dashboard — Vue 3 SPA embedded via `rust-embed`, served at `/nvr/`     |

## Quick Start

### Prerequisites

- **Rust** (edition 2024)
- **FFmpeg 7.x** shared libraries (headers + libs)
- **ZLMediaKit** (RTSP/RTMP/HLS distribution; always compiled in)

### 1. Install dependencies

```bash
# Auto-download FFmpeg & ZLMediaKit for your platform
bash scripts/pre_install_deps.sh
```

On macOS this script installs `ffmpeg@7` via Homebrew and creates symlinks under
`./ffmpeg/`.

### 2. Build & run

```bash
# Build all crates
cargo build --workspace

# Run the main app (API server on :18080)
cargo run --package nvr
```

The API server listens on `http://localhost:18080` by default, with all routes
mounted under the `/api` prefix.

The web dashboard is available at `http://localhost:18080/nvr/`.

ZLMediaKit serves streaming on its own ports:

| Protocol | Port |
| -------- | ---- |
| HTTP / HLS | `8553` |
| RTSP     | `8554` |
| RTMP     | `8555` |

> A `Makefile` is provided that exports `FFMPEG_DIR`, `ZLM_DIR`, and
> `LD_LIBRARY_PATH` for you. Run `make help` to list targets (`make run`,
> `make build`, `make test`, `make watch`, …).

### 3. Create a pipeline

```bash
# RTSP input → ZLMediaKit output
curl -X POST http://localhost:18080/api/pipe/add \
  -H "Content-Type: application/json" \
  -d '{
    "id": "cam1",
    "input": { "t": "net", "i": "rtsp://192.168.1.100:554/stream" },
    "outputs": [{
      "t": "zlm",
      "zlm": { "app": "live", "stream": "cam1" }
    }]
  }'
```

Play the stream:

```bash
# Via RTSP (ZLMediaKit)
ffplay rtsp://127.0.0.1:8554/live/cam1

# Or via HLS (wait a few seconds for the first segments)
ffplay http://127.0.0.1:8553/live/cam1/hls.m3u8
```

## REST API

All endpoints are mounted under `/api`.

### Authentication

Every endpoint except `POST /api/user/login` requires a session token,
passed either as an `Authorization: Bearer <token>` header or a `?token=`
query parameter (the query form exists for HLS players that cannot set
headers; playlist endpoints propagate it into the segment URIs they emit).
Requests without a valid token get `401`.

Tokens are issued by `POST /api/user/login`, persisted server-side (they
survive restarts), and expire 30 days after login. A default `admin`/`admin`
user is created on first start — change its password after deployment.

```bash
TOKEN=$(curl -s -X POST http://localhost:18080/api/user/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "admin"}' | jq -r .data.token)

curl -H "Authorization: Bearer $TOKEN" http://localhost:18080/api/device/list
```

### Pipelines — `/api/pipe`

The pipeline API manages ephemeral, in-memory media pipelines.

| Method | Endpoint                | Description           |
| ------ | ----------------------- | --------------------- |
| GET    | `/api/pipe/list`        | List active pipeline IDs |
| POST   | `/api/pipe/add`         | Create a new pipeline |
| GET    | `/api/pipe/remove/{id}` | Remove a pipeline     |
| GET    | `/api/pipe/status/{id}` | Get pipeline status   |

#### Input types

The `input` object takes a type tag `t` and an input value `i`:

| Type          | `t`       | Example `i`                               |
| ------------- | --------- | ----------------------------------------- |
| Network       | `net`     | `rtsp://host:554/path`                    |
| File          | `file`    | `/path/to/video.mp4`                      |
| Screen (X11)  | `x11grab` | `:99`                                     |
| Test pattern  | `lavfi`   | `testsrc=size=1920x1080:rate=10,realtime` |
| V4L2 device   | `v4l2`    | `/dev/video0`                             |

#### Output types

Each entry in `outputs` takes a type tag `t`:

| Type        | `t`        | Config                                                  |
| ----------- | ---------- | ------------------------------------------------------- |
| ZLMediaKit  | `zlm`      | `{ "app": "live", "stream": "cam1" }`                   |
| Network     | *(default)*| `{ "net": { "url": "rtsp://…", "format": "rtsp" } }`    |

#### Encode options (optional, per output)

```json
{
  "encode": {
    "preset": "ultrafast",
    "bitrate": 2000000
  }
}
```

- `preset` — x264 preset: `ultrafast` (default, fastest), `superfast`, `veryfast`, `fast`, `medium`, … (slower = better quality)
- `bitrate` — target bitrate in bps

### Devices — `/api/device`

Devices are persisted, dashboard-managed sources. Adding a device stores it in
the database and automatically starts a pipeline that publishes to ZLMediaKit.

| Method | Endpoint                  | Description       |
| ------ | ------------------------- | ----------------- |
| GET    | `/api/device/list`        | List devices (with FLV URLs) |
| POST   | `/api/device/add`         | Add a device      |
| POST   | `/api/device/update/{id}` | Update a device   |
| POST   | `/api/device/remove/{id}` | Remove a device   |

```bash
curl -X POST http://localhost:18080/api/device/add \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Front Door",
    "input_type": "net",
    "input_value": "rtsp://192.168.1.100:554/stream",
    "include_audio": true
  }'
```

`include_audio` toggles whether the device audio track is forwarded to its ZLM
live stream.

### Playback — `/api/playback`

Recorded HLS segments are persisted and exposed for playback.

| Method | Endpoint                                   | Description                  |
| ------ | ------------------------------------------ | ---------------------------- |
| GET    | `/api/playback/device/list`                | List devices with recordings |
| GET    | `/api/playback/device/{device_id}/segments`| List recorded segments       |
| GET    | `/api/playback/device/{device_id}/today`   | List today's segments        |
| GET    | `/api/playback/playlist/{device_id}`       | Build a playback playlist    |
| GET    | `/api/playback/segment/{id}`               | Play a single segment        |
| POST   | `/api/playback/segment/{id}/delete`        | Delete one segment           |
| POST   | `/api/playback/segments/delete`            | Delete segments (`{ "ids": [...] }`) |
| POST   | `/api/playback/device/{device_id}/segments/delete` | Delete all of a device's segments |

> The REST API uses only **GET** and **POST** — mutations (update/remove/delete)
> go through POST with a verb in the path.

### System — `/api/system`

| Method | Endpoint                          | Description                  |
| ------ | --------------------------------- | ---------------------------- |
| GET    | `/api/system/list/device/formats` | List supported device formats |
| GET    | `/api/system/list/v4l2/devices`   | List available V4L2 devices  |
| GET    | `/api/system/list/x11grab/devices`| List available X11 displays  |

### User — `/api/user`

| Method | Endpoint                       | Description                                  |
| ------ | ------------------------------ | -------------------------------------------- |
| POST   | `/api/user/login`              | Log in → `{ token, username }` (no auth)     |
| POST   | `/api/user/logout`             | Revoke the current session                   |
| GET    | `/api/user/info`               | Current user info                            |
| POST   | `/api/user/password`           | Change own password (`{ old_password, new_password }`); kicks the user's other sessions |
| GET    | `/api/user/list`               | List users                                   |
| POST   | `/api/user/add`                | Create a user (`{ username, password }`)     |
| POST   | `/api/user/remove/{username}`  | Delete a user (not yourself) and revoke their sessions |

## Development

### Testing

```bash
# Run all tests (no binary harness)
cargo test --workspace --lib --tests --no-fail-fast

# Run tests for a single crate
cargo test -p nvr
cargo test -p ffmpeg-bus

# Format code
cargo fmt

# Fast compile check
cargo check --workspace
```

Tests are colocated as `*_test.rs` files alongside the source they cover (e.g.
`ffmpeg-bus/src/bus.rs` → `ffmpeg-bus/src/bus_test.rs`).

### Frontend development

```bash
cd nvr-dashboard/app

# Install dependencies
npm ci

# Dev server with hot reload
npm run dev

# Build for production (embedded into the Rust binary)
npm run build

# Type check and lint
npm run type-check
npm run lint
```

The dashboard is automatically built by `build.rs` when `app/dist` is missing or
the source files change, so a manual build is usually unnecessary.

## Environment Variables

| Variable          | Description                                                            |
| ----------------- | --------------------------------------------------------------------- |
| `RUST_LOG`        | Log level filter (e.g. `info`, `debug`, `ffmpeg_bus=debug`)           |
| `FFMPEG_DIR`      | Path to the FFmpeg installation (default: `./ffmpeg`)                 |
| `ZLM_DIR`         | Path to the ZLMediaKit installation (default: `./zlm`)               |
| `LD_LIBRARY_PATH` | Runtime library path; must include `ffmpeg/lib` and `zlm/lib`        |

## Configuration

Runtime configuration is stored in the `kvs` table of the SQLite database (WAL
mode). Schema migrations live in `nvr-db/migrations/` and are embedded into the
binary.

## License

MIT
