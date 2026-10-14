#!/usr/bin/env bash
# Download the FFmpeg 8.1 shared RK SDK; print its absolute directory on stdout.
set -euo pipefail
cd "$(dirname "$0")/.."
deps="$PWD/target/cross-deps/rockchip/ffmpeg"
url="${RK_FFMPEG_URL:-https://github.com/BenLocal/FFmpeg-Builds/releases/download/latest/ffmpeg-d90e3a1c18-latest-linuxarm64-gpl-shared-8.1-rk.tar.xz}"
archive="$deps/../ffmpeg.tar.xz"
expected="${RK_FFMPEG_SHA256:-}"
if [[ -z "$expected" ]]; then
    expected=$(curl -fsSL --retry 3 "${url%/*}/checksums.sha256" |
        awk -v name="${url##*/}" '$2 == name || $2 == "*" name { print $1 }')
fi
if [[ ! "$expected" =~ ^[[:xdigit:]]{64}$ ]]; then
    echo "Missing or invalid SHA-256 for ${url##*/}" >&2
    exit 1
fi

# The floating latest asset can change without changing its filename/version.
if [[ -f "$deps/include/libavcodec/avcodec.h" && -f "$deps/.archive-sha256" &&
    "$(cat "$deps/.archive-sha256")" == "$expected" ]]; then
    printf '%s\n' "$deps"
    exit 0
fi

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    else
        shasum -a 256 "$1" | awk '{ print $1 }'
    fi
}

mkdir -p "${deps%/*}"
stage=$(mktemp -d "${deps%/*}/ffmpeg-download.XXXXXX")
cleanup() {
    if [[ ! -e "$deps" && -d "$stage/previous" ]]; then
        mv "$stage/previous" "$deps"
    fi
    rm -rf "$stage"
}
trap cleanup EXIT

if [[ ! -f "$archive" || "$(sha256 "$archive")" != "$expected" ]]; then
    curl -fL --retry 3 -o "$stage/ffmpeg.tar.xz" "$url" >&2
    if [[ "$(sha256 "$stage/ffmpeg.tar.xz")" != "$expected" ]]; then
        echo "Rockchip FFmpeg archive SHA-256 mismatch" >&2
        exit 1
    fi
    mv "$stage/ffmpeg.tar.xz" "$archive"
fi

mkdir "$stage/sdk"
tar -xJf "$archive" -C "$stage/sdk" --strip-components=1
test -f "$stage/sdk/include/libavcodec/avcodec.h"
test -f "$stage/sdk/lib/libavdevice.so"
test -x "$stage/sdk/bin/ffmpeg"
printf '%s\n' "$expected" > "$stage/sdk/.archive-sha256"
if [[ -e "$deps" ]]; then mv "$deps" "$stage/previous"; fi
mv "$stage/sdk" "$deps"
printf '%s\n' "$deps"
