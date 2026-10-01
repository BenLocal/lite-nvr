# lite-nvr

A lightweight Network Video Recorder built with Rust. It ingests video, transcodes
it with FFmpeg, and distributes it through
[ZLMediaKit](https://github.com/ZLMediaKit/ZLMediaKit) (RTSP / RTMP / HLS / FLV),
with a REST API and an embedded Vue 3 dashboard.

## Features

- **Inputs**: RTSP/RTMP, local files, screen capture (x11grab), V4L2, test patterns, GB28181, ONVIF, Xiaomi cameras, live-platform pages (via yt-dlp)
- **Processing**: FFmpeg transcoding, seamless director switching, multi-view compositing, audio mixing
- **Recording**: time-sliced segments with playback, retention cleanup, FTP/SMB offload
- **AI**: real-time speech-to-text (sherpa-onnx) and object detection (YOLO / ONNX Runtime)
- **Ops**: session auth, SQLite persistence, system metrics, web dashboard

## Quick Start

Requires Rust (edition 2024), Node.js/npm (the dashboard is built automatically by `build.rs`) and FFmpeg 7.x+ shared libraries.

```bash
make install-deps   # download FFmpeg & ZLMediaKit into ./ffmpeg and ./zlm
make run            # = cargo run --package nvr, with LD_LIBRARY_PATH set up
```

| Service | Address |
| --- | --- |
| REST API | `http://localhost:18080/api` |
| Dashboard | `http://localhost:18080/nvr/` |
| ZLM HTTP / HLS | `:8553` |
| ZLM RTSP | `:8554` |
| ZLM RTMP | `:8555` |

A default `admin` / `admin` user is created on first start — change the password after deployment.

## Example

Every endpoint except `POST /api/user/login` needs a token (`Authorization: Bearer <token>` or `?token=`).

```bash
TOKEN=$(curl -s -X POST http://localhost:18080/api/user/login \
  -H "Content-Type: application/json" \
  -d '{"username": "admin", "password": "admin"}' | jq -r .data.token)

# RTSP camera → ZLMediaKit
curl -X POST http://localhost:18080/api/pipe/add \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{
    "id": "cam1",
    "input": { "t": "net", "i": "rtsp://192.168.1.100:554/stream" },
    "outputs": [{ "t": "zlm", "zlm": { "app": "live", "stream": "cam1" } }]
  }'

ffplay rtsp://127.0.0.1:8554/live/cam1
```

More requests are in [`rest/api.rest`](rest/api.rest).

## Development

```bash
make help                                             # list all targets
cargo check --workspace
cargo test --workspace --lib --tests --no-fail-fast
cd nvr-dashboard/app && npm run dev                   # dashboard dev server
```

Project conventions, architecture notes and environment variables are in [`AGENTS.md`](AGENTS.md) and [`docs/agents-dot-md/`](docs/agents-dot-md/).

## License

MIT
