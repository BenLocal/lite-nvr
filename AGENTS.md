# Repository Guidelines

## Project Structure & Module Organization

This repository is a Rust workspace with four crates listed in `Cargo.toml`:

- `nvr-server/`: main service entrypoint, REST API, pipeline orchestration, and ZLMediaKit integration
- `ffmpeg-bus/`: FFmpeg-based media pipeline primitives
- `nvr-db/`: SQLite/Turso access and migrations
- `nvr-dashboard/`: embedded dashboard service, with the Vue app in `nvr-dashboard/app/`

Rust sources live under each crate's `src/`. Media-related tests are colocated, for example `ffmpeg-bus/src/*_test.rs` and `nvr-server/src/media/pipe_test.rs`. Helper scripts are in `scripts/`. CI workflows and local editor tasks live in `.github/workflows/` and `.vscode/`.

## Build, Test, and Development Commands

- `bash scripts/pre_install_deps.sh`: install FFmpeg and ZLMediaKit prerequisites
- `cargo build --workspace`: build all Rust crates
- `cargo run --package nvr-server`: start the API server on `:8080`
- `cargo check --workspace`: fast compile check across the workspace
- `cargo test --workspace --lib --tests --no-fail-fast`: run workspace tests without the binary harness
- `cargo test -p nvr-server`: run service crate tests only
- `cd nvr-dashboard/app && npm ci && npm run build`: rebuild embedded frontend assets
- `cd nvr-dashboard/app && npm run dev`: run the Vite dashboard locally

## Coding Style & Naming Conventions

Use `cargo fmt` for Rust formatting and idiomatic snake_case for modules, files, and functions. Keep crate boundaries clean; avoid cross-crate utility dumps. In the dashboard, use TypeScript, Vue Composition API, and PascalCase component filenames such as `LoginView.vue`. Prefer targeted, minimal changes over broad refactors.

## Testing Guidelines

Place tests next to the code they validate. Favor focused unit tests for config parsing, pipeline assembly, and handler behavior. Run the narrowest relevant test first, then the broader workspace checks. Frontend changes should at least pass `npm run build`; add manual verification notes for UI changes.

## Commit & Pull Request Guidelines

Recent history follows Conventional Commits, for example `fix(rust-check): update cargo test command to skip bin harness`. Keep subjects imperative and scoped by subsystem when useful. PRs should include: problem statement, key implementation notes, commands run, linked issues, and screenshots for dashboard changes.

## Configuration Tips

FFmpeg shared libraries are required at runtime. Set `FFMPEG_DIR` when using a non-default install, and ensure `LD_LIBRARY_PATH` includes `ffmpeg/lib` and `zlm/lib` for local debugging.
