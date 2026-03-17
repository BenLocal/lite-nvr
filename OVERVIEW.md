# nvr-server Project Overview

## Overall Summary
This is a lightweight NVR project built as a Rust workspace. It follows a layered structure of "main service orchestration + media processing engine + database layer + web dashboard". The backend processes video streams through an FFmpeg pipeline and can integrate with ZLMediaKit for distribution.

## Top-Level Structure
1. The workspace is defined in `Cargo.toml` and contains 4 crates: `nvr-server`, `ffmpeg-bus`, `nvr-db`, and `nvr-dashboard`.
2. `README.md` provides the architecture diagram, API examples, and quick start instructions.
3. The repository includes `scripts/` (dependency setup and helper scripts), `rest/api.rest` (API test examples), and `nvr.db` (local sqlite data file).

## Module Responsibilities
### 1. nvr-server (main application)
- Entry point: `nvr-server/src/main.rs`
- Responsibility: initialize logging and FFmpeg, run database migrations, start API server (port 8080), and optionally start the ZLM server.
- API assembly: `nvr-server/src/api.rs`, mounting `/user`, `/pipe`, `/system`, and `/nvr`.
- Management layer: `nvr-server/src/manager.rs`, using a global `RwLock<HashMap<String, Arc<Pipe>>>` to manage pipeline lifecycles.
- Media pipeline: `nvr-server/src/media/pipe.rs`, converting business config into ffmpeg-bus input/output and running async processing.

### 2. ffmpeg-bus (media core)
- Entry point: `ffmpeg-bus/src/lib.rs`
- Responsibility: encapsulate FFmpeg workflows and provide modules such as input/decoder/encoder/output/bus as the media processing engine.

### 3. nvr-db (database layer)
- Migration logic: `nvr-db/src/migrations.rs`; migration SQL in `nvr-db/migrations/20260210_init.sql`.
- Database setup: `nvr-db/src/db.rs`, based on turso/local sqlite with WAL enabled.
- Data access: `nvr-db/src/kv.rs`; current storage access mainly uses the kv table.

### 4. nvr-dashboard (frontend and embedded static assets)
- Rust side: `nvr-dashboard/src/lib.rs` uses `rust-embed` to serve `app/dist` static files.
- Frontend: `nvr-dashboard/app` (Vue3 + Vite + PrimeVue), with routes in `nvr-dashboard/app/src/router/index.ts`.

## Core Runtime Flow
1. Client sends `POST /pipe/add` (`nvr-server/src/handler/media_pipe.rs`).
2. The request is parsed into `PipeConfig`, then passed to `manager::add_pipe`.
3. `Pipe::start` creates an ffmpeg-bus instance, registers input/output, and forwards stream data by destination type (network/ZLM/sink).
4. During app startup, database migrations run first and then the global DB connection entry is initialized.

## Current Status Assessment
1. The layering is clear, and the core media path has well-defined module boundaries.
2. The frontend is currently a basic shell (device page is not yet connected to real data).
3. Password verification logic in login is not enabled yet (currently returns a generated token).
4. Minor naming/spelling issues have been cleaned up (for example, `media_pipe_router` and `formats`).
