# lite-nvr

A lightweight Network Video Recorder built with Rust. It provides a media pipeline that ingests video from various sources (RTSP, files, screen capture, test patterns), transcodes via FFmpeg, and pushes to [ZLMediaKit](https://github.com/ZLMediaKit/ZLMediaKit) for RTSP/RTMP/HLS distribution.

## Architecture

```
┌──────────┐    ┌───────────┐    ┌─────────┐    ┌─────────────┐
│  Input   │───▶│  Decoder  │───▶│ Encoder │───▶│   Output    │
│ (source) │    │ (ffmpeg)  │    │(libx264)│    │(ZLM/file/…) │
└──────────┘    └───────────┘    └─────────┘    └─────────────┘
```

## Workspace Crates

| Crate             | Description                                                                         |
| ----------------- | ----------------------------------------------------------------------------------- |
| **lite-nvr**      | Main application — REST API, pipeline management, ZLMediaKit integration            |
| **ffmpeg-bus**    | Core media bus — input demux, decoder, encoder, muxer, all wired via async channels |
| **nvr-db**        | Database layer with migrations (SQLite)                                             |
| **nvr-dashboard** | Web dashboard frontend                                                              |

## Prerequisites

- **Rust** (edition 2024)
- **FFmpeg 7.x** shared libraries (headers + libs)
- **ZLMediaKit** (optional, enabled by default via `zlm` feature)

## Quick Start

### 1. Install dependencies

```bash
# Auto-download FFmpeg & ZLMediaKit for your platform
bash scripts/pre_install_deps.sh
```

On macOS this installs `ffmpeg@7` via Homebrew and creates symlinks into `./ffmpeg/`.

### 2. Build & Run

```bash
cargo run --package lite-nvr
```

The API server starts on `http://localhost:8080` by default.

### 3. Create a pipeline

```bash
# RTSP input → ZLMediaKit output
curl -X POST http://localhost:8080/pipe/add \
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

Then play via:

```bash
ffplay rtsp://127.0.0.1:8554/live/cam1
```

## REST API

| Method | Endpoint            | Description           |
| ------ | ------------------- | --------------------- |
| GET    | `/pipe/list`        | List active pipelines |
| POST   | `/pipe/add`         | Create a new pipeline |
| GET    | `/pipe/remove/{id}` | Remove a pipeline     |
| GET    | `/pipe/status/{id}` | Get pipeline status   |

### Input types

| Type                | `t` field | `i` field example                         |
| ------------------- | --------- | ----------------------------------------- |
| Network (RTSP/RTMP) | `net`     | `rtsp://host:554/path`                    |
| File                | `file`    | `/path/to/video.mp4`                      |
| Screen capture      | `x11grab` | `:99`                                     |
| Test pattern        | `lavfi`   | `testsrc=size=1920x1080:rate=10,realtime` |
| V4L2 device         | `v4l2`    | `/dev/video0`                             |

### Output types

| Type       | `t` field | Config                                                 |
| ---------- | --------- | ------------------------------------------------------ |
| ZLMediaKit | `zlm`     | `{ "app": "live", "stream": "cam1" }`                  |
| Network    | (default) | `{ "net": { "url": "rtsp://...", "format": "rtsp" } }` |

### Encode options (optional)

```json
{
  "encode": {
    "preset": "ultrafast",
    "bitrate": 2000000
  }
}
```

Presets: `ultrafast`, `superfast`, `veryfast`, `fast`, `medium` (slower = better quality).

## Environment Variables

| Variable     | Description                                                 |
| ------------ | ----------------------------------------------------------- |
| `RUST_LOG`   | Log level filter (e.g. `info`, `debug`, `ffmpeg_bus=debug`) |
| `FFMPEG_DIR` | Path to FFmpeg installation (default: `./ffmpeg`)           |

## License

MIT
