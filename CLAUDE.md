# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Summary

Lightweight NVR (Network Video Recorder) built in Rust. Ingests video from RTSP, files, screen capture, test patterns, and V4L2 devices, transcodes via FFmpeg, and distributes through ZLMediaKit (RTSP/RTMP/HLS). Includes a REST API server and Vue 3 web dashboard.

## Build & Development Commands

```bash
# Install FFmpeg & ZLMediaKit prerequisites
bash scripts/pre_install_deps.sh

# Build all crates
cargo build --workspace

# Run the main application (API server on :8080)
cargo run --package nvr

# Fast compile check
cargo check --workspace

# Run all tests (no binary harness)
cargo test --workspace --lib --tests --no-fail-fast

# Run tests for a single crate
cargo test -p nvr
cargo test -p ffmpeg-bus

# Format code
cargo fmt

# Build frontend (automatically built by nvr-dashboard build.rs)
# Manual build if needed:
cd nvr-dashboard/app && npm ci && npm run build

# Frontend dev server
cd nvr-dashboard/app && npm run dev

# Frontend lint & type check
cd nvr-dashboard/app && npm run lint
cd nvr-dashboard/app && npm run type-check
```

### Environment Variables

- `FFMPEG_DIR` — path to FFmpeg installation (default: `./ffmpeg`)
- `ZLM_DIR` — path to ZLMediaKit installation (default: `./zlm`)
- `LD_LIBRARY_PATH` — must include `ffmpeg/lib` and `zlm/lib` for local runs
- `RUST_LOG` — log level filter (e.g. `info`, `ffmpeg_bus=debug`)

## Architecture

**Workspace crates:**

| Crate | Role |
|-------|------|
| `nvr` | Main app — REST API (Axum), pipeline lifecycle, ZLMediaKit integration |
| `ffmpeg-bus` | Media engine — input demux, decoder, encoder, muxer wired via async channels |
| `nvr-db` | Database layer — SQLite/Turso with embedded SQL migrations, KV store |
| `nvr-dashboard` | Vue 3 SPA embedded via `rust-embed`, served at `/nvr/`. Frontend is automatically built by `build.rs` when `app/dist` is missing or source files change. |

**Runtime data flow:**

```
REST API request → Handler → Manager (global RwLock<HashMap>) → Pipe → ffmpeg-bus Bus
                                                                         ├→ Input (demux)
                                                                         ├→ Decoder
                                                                         ├→ Encoder (libx264)
                                                                         └→ Output (ZLM/file/network/sink)
```

**Key entry points:**
- `nvr/src/main.rs` — app init: FFmpeg, migrations, ZLM server, device pipelines, API server
- `nvr/src/api.rs` — route mounting (`/user`, `/pipe`, `/device`, `/playback`, `/system`, `/nvr`)
- `nvr/src/manager.rs` — pipeline lifecycle via global `RwLock<HashMap<String, Arc<Pipe>>>`
- `nvr/src/media/pipe.rs` — translates business config into ffmpeg-bus operations
- `ffmpeg-bus/src/bus.rs` — core media command dispatcher (~35KB, most complex file)

**Database:** SQLite with WAL mode. Migrations in `nvr-db/migrations/`. Uses a KV table (`kvs`) for flexible config storage.

## Conventions

- **Rust:** `cargo fmt`, snake_case for modules/files/functions, Rust edition 2024
- **Tests:** colocated as `*_test.rs` files alongside source files. For example:
  - `ffmpeg-bus/src/bus.rs` → `ffmpeg-bus/src/bus_test.rs`
  - `nvr/src/media/pipe.rs` → `nvr/src/media/pipe_test.rs`
  - Keep test files in the same directory as the code they test, using the `_test.rs` suffix
  - Import test modules at the end of source files using:
    ```rust
    #[cfg(test)]
    #[path = "module_name_test.rs"]
    mod module_name_test;
    ```
- **Frontend:** TypeScript, Vue Composition API, PascalCase component filenames, PrimeVue UI library
- **Commits:** Conventional Commits format (e.g. `fix(rust-check): update cargo test command`)
- **Dashboard build output** (`nvr-dashboard/app/dist/`) is embedded into the Rust binary via `rust-embed`
