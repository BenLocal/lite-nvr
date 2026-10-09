#!/usr/bin/env bash
# Download the FFmpeg 8.1 shared RK SDK; print its absolute directory on stdout.
set -euo pipefail
cd "$(dirname "$0")/.."
deps="$PWD/target/cross-deps/rockchip/ffmpeg"
if [[ ! -f "$deps/include/libavcodec/avcodec.h" ]]; then
    mkdir -p "$deps"
    url="${RK_FFMPEG_URL:-https://github.com/BenLocal/FFmpeg-Builds/releases/download/latest/ffmpeg-d90e3a1c18-latest-linuxarm64-gpl-shared-8.1-rk.tar.xz}"
    curl -fL --retry 3 -o "$deps/../ffmpeg.tar.xz" "$url" >&2
    tar -xJf "$deps/../ffmpeg.tar.xz" -C "$deps" --strip-components=1
fi
printf '%s\n' "$deps"
