# Repository Guidelines

## Project Structure & Module Organization
This repository is a Rust workspace with four crates listed in `Cargo.toml`: `nvr/` (main API and process entrypoint), `ffmpeg-bus/` (media pipeline primitives), `nvr-db/` (SQLite access and migrations), and `nvr-dashboard/` (embedded frontend service). The Vue dashboard source lives in `nvr-dashboard/app/src/`, while the Rust wrapper for serving built assets is in `nvr-dashboard/src/`. Architecture notes and troubleshooting live in `docs/`. Helper scripts, including dependency bootstrap, live in `scripts/`.

## Build, Test, and Development Commands
Run `bash scripts/pre_install_deps.sh` once on a fresh machine to prepare FFmpeg and ZLMediaKit dependencies. Use `cargo build` to compile the full workspace and `cargo run --package lite-nvr` to start the API server on `:8080`. Run `cargo test` for workspace tests; narrow scope with `cargo test -p lite-nvr` when iterating on one crate. For the dashboard, work in `nvr-dashboard/app`: `npm install`, `npm run dev` for Vite local development, `npm run build` to emit `app/dist/`, and `npm run lint` to apply ESLint fixes. Rebuild the frontend before shipping Rust binaries because `nvr-dashboard` embeds `app/dist/` at compile time.

## Coding Style & Naming Conventions
Follow standard Rust formatting with `cargo fmt` before review and use idiomatic snake_case for modules, files, and functions. Prefer small modules under `src/` and keep crate boundaries clear instead of cross-cutting utility dumps. In the Vue app, use TypeScript, Composition API, and PascalCase component filenames such as `DeviceListView.vue`; keep route and shared utility names descriptive and stable. ESLint is the frontend formatter/linter (`npm run lint` and `npm run format` currently run the same fix command).

## Testing Guidelines
Rust tests are primarily inline `#[test]` modules and `*_test.rs` files such as `nvr/src/media/pipe_test.rs` and `ffmpeg-bus/src/bus_test.rs`. Add tests next to the code they validate and cover builder logic, config parsing, and media flow edge cases when behavior changes. There is no dedicated frontend test suite in this repo today, so at minimum run `npm run lint`, `npm run build`, and verify affected screens manually.

## Commit & Pull Request Guidelines
Recent history follows concise Conventional Commit-style subjects: `feat: ...`, `fix: ...`, `refactor: ...`, `docs: ...`. Keep commits focused by subsystem when possible, and write imperative summaries that explain the user-visible or architectural change. PRs should include a short problem statement, key implementation notes, commands run (`cargo test`, `npm run build`, etc.), linked issues, and screenshots for dashboard changes.
