# FFmpeg Rockchip compatibility patch

`ffmpeg-rockchip-8.1.0.patch` targets the crates.io `ffmpeg-next` and
`ffmpeg-sys-next` **8.1.0** sources and changes three files:

- `ffmpeg-next/build.rs`: detect the RK SDK additional pixel formats.
- `ffmpeg-next/src/util/format/pixel.rs`: map NV15 and packed NV20 (`NV20RK`) both ways while retaining the generic NV20 alias.
- `ffmpeg-sys-next/build.rs`: regenerate bindings/link paths when FFMPEG_DIR changes.

Rockchip package builds and the project build wrapper automatically prepare pristine
sources, apply this patch and cache the result under `.cache/rockchip-rust/`.
Upstream licenses remain in the cached sources. No `vendor/` directory is needed.
Ordinary builds use crates.io dependencies without these overrides.

Cargo cannot select `[patch.crates-io]` by feature. For manual Rockchip commands,
use the wrapper, which supplies temporary workspace Cargo path overrides and restores
`.cargo/config.toml` and Cargo.lock after the command finishes:

```bash
bash scripts/with-rockchip-patch.sh cargo check -p ffmpeg-bus --features rockchip
bash scripts/with-rockchip-patch.sh cross build --release --locked -p nvr --target aarch64-unknown-linux-gnu --features rockchip
```

Set FFMPEG_DIR to the RK SDK before these manual commands. Run builds sequentially
because the wrapper temporarily updates the workspace lockfile. The cache is keyed
by patch SHA256; `prepare-rockchip-patch.sh` reuses Cargo registry sources when
available and otherwise downloads the original crate archives using proxy environment variables.
