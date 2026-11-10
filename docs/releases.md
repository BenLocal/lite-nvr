# Offline releases

The Release workflow builds three archives:

| Archive suffix | Target |
| --- | --- |
| `linux-amd64-gnu.tar.gz` | Linux x86_64 |
| `linux-arm64-gnu.tar.gz` | Linux ARM64 |
| `linux-arm64-rockchip.tar.gz` | Rockchip ARM64, with RK FFmpeg and NPU runtimes |

Each archive includes the NVR executable, embedded dashboard, FFmpeg/ffprobe,
ZLMediaKit, ONNX Runtime CPU, ALSA configuration and their non-glibc shared-library dependencies.
Sherpa's native libraries are linked into the executable. Rockchip archives also
include MPP/RGA, RKNN and RKLLM user-space libraries. Pinned Rockchip runtime
downloads are checked against SHA-256 values and recorded in `DEPENDENCIES.json`.

Model files, model resources, yt-dlp and development tools are excluded.
Model-dependent features remain disabled until models are configured.

The target needs GNU Linux with glibc 2.35 or newer. The operating system supplies
glibc and the ELF loader. Hardware features need compatible kernel drivers,
devices and permissions; X11 capture also needs a running display server.
No package installation or Internet access is required to start the NVR.

## Install

Download the archive for your machine and `SHA256SUMS` from GitHub Releases.
Check the matching checksum, unpack the archive into a writable directory, then
run its launcher:

```bash
sha256sum --ignore-missing -c SHA256SUMS
tar -xzf lite-nvr-0.1.0-linux-amd64-gnu.tar.gz
cd lite-nvr-0.1.0-linux-amd64-gnu
./start.sh
```

Open `http://localhost:18080/nvr/`. The initial account is `admin` / `admin`;
change its password after signing in. Database and recording files live in the
package directory by default. Keep them when replacing an installation.

All launchers also accept `start`, `stop`, `restart` and `status`. See
[Rockchip deployment](rockchip.md) for board setup and device access.

## Build locally

```bash
make package PACKAGE_ARCHS="amd64 arm64 rockchip" PACKAGE_ARCHIVE=1 PACKAGE_OFFLINE=1
```

The build machine needs Rust/rustup, cross 0.2.5, Docker, Python 3, Node.js/npm,
curl, tar and xz. Dependency downloads happen on the build machine.
On Apple Silicon, the build container uses ARM64 Linux tools even when producing
x86_64 binaries. Each target retains its own Cargo cache under `target/package/`.

Verify a package on a machine or Docker engine supporting its architecture:

```bash
bash scripts/tests/verify-offline-package.sh dist/lite-nvr-0.1.0-linux-arm64-rockchip
```

Verification uses an Ubuntu 22.04 base container with `--network none`. It checks
shared-library resolution, FFmpeg/ffprobe execution, NVR startup and an HTTP 200
response from the embedded dashboard. It does not exercise hardware or inference
without devices and models.

## Publish

Set the version in `nvr/Cargo.toml`, commit the release changes, then push a
matching tag such as `v0.1.0`. The workflow rejects tags that do not match that
version. Native x86_64 and ARM64 GitHub runners build the three packages and test
each extracted archive without networking. Only when all three pass does the
workflow create a GitHub Release with the archives, `SHA256SUMS` and generated
release notes. It needs the standard `GITHUB_TOKEN` with `contents: write` in the
publish job; no separate publishing secret is required.

Run the workflow manually to build and verify downloadable Actions artifacts
without creating a GitHub Release.
