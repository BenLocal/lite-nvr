#!/usr/bin/env bash
# Build the lite-nvr workspace with the same env wiring as the Makefile.
#
#   build.sh [check|build|release|env] [extra cargo args...]
#
# `source build.sh env` exports the environment into the current shell only.

_nvr_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

_nvr_setup_env() {
    if [ -f "$_nvr_root/.env" ]; then
        set -a
        # shellcheck disable=SC1091
        . "$_nvr_root/.env"
        set +a
    fi
    # curl/wget only honor lowercase proxy vars for http URLs.
    [ -n "${HTTPS_PROXY:-}" ] && export https_proxy="${https_proxy:-$HTTPS_PROXY}"
    [ -n "${HTTP_PROXY:-}" ] && export http_proxy="${http_proxy:-$HTTP_PROXY}"

    # Prefer a local ./ffmpeg (make install-deps); otherwise leave FFMPEG_DIR
    # unset so ffmpeg-sys-next falls back to the system FFmpeg via pkg-config.
    if [ -z "${FFMPEG_DIR:-}" ] && [ -d "$_nvr_root/ffmpeg/lib" ]; then
        export FFMPEG_DIR="$_nvr_root/ffmpeg"
    fi
    if [ -n "${FFMPEG_DIR:-}" ]; then
        export FFMPEG_DIR
        export LD_LIBRARY_PATH="$FFMPEG_DIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    fi
    export RUST_LOG="${RUST_LOG:-info}"

    if [ -z "${ZLM_DIR:-}" ] && [ -d "$_nvr_root/zlm" ]; then
        export ZLM_DIR="$_nvr_root/zlm"
    fi
    if [ -n "${ZLM_DIR:-}" ]; then
        export LD_LIBRARY_PATH="$LD_LIBRARY_PATH:$ZLM_DIR/lib"
    fi

    # An empty SHERPA_ONNX_LIB_DIR breaks sherpa-onnx-sys, so only set it when
    # prebuilt libs actually exist.
    if [ -z "${SHERPA_ONNX_LIB_DIR:-}" ]; then
        unset SHERPA_ONNX_LIB_DIR
        for d in "$_nvr_root"/third_party/sherpa-onnx/*/lib; do
            if [ -d "$d" ]; then
                export SHERPA_ONNX_LIB_DIR="$d"
                break
            fi
        done
    fi
}

_nvr_check_prereqs() {
    local ok=1
    if [ -n "${FFMPEG_DIR:-}" ]; then
        if [ ! -d "$FFMPEG_DIR/lib" ]; then
            echo "error: FFMPEG_DIR=$FFMPEG_DIR has no lib/ (fix it or run: make install-deps)" >&2
            ok=0
        fi
    elif pkg-config --exists libavcodec 2>/dev/null; then
        echo "info: using system FFmpeg $(pkg-config --modversion libavcodec) via pkg-config" >&2
    else
        echo "error: no FFmpeg found (run: make install-deps, or set FFMPEG_DIR)" >&2
        ok=0
    fi
    if ! command -v cargo >/dev/null 2>&1; then
        echo "error: cargo not found in PATH" >&2
        ok=0
    fi
    if ! command -v npm >/dev/null 2>&1; then
        echo "warn: npm not found; nvr-dashboard/build.rs will fail if the frontend needs rebuilding" >&2
    fi
    [ "$ok" = 1 ]
}

_nvr_setup_env

if [ "${1:-}" = "env" ]; then
    echo "FFMPEG_DIR=${FFMPEG_DIR:-<system pkg-config>}"
    echo "ZLM_DIR=${ZLM_DIR:-<auto-download by rszlm-sys>}"
    echo "LD_LIBRARY_PATH=$LD_LIBRARY_PATH"
    echo "SHERPA_ONNX_LIB_DIR=${SHERPA_ONNX_LIB_DIR:-<auto-download by sherpa-onnx-sys>}"
    # shellcheck disable=SC2317
    return 0 2>/dev/null || exit 0
fi

set -euo pipefail

mode="build"
case "${1:-}" in
    check | build | release)
        mode="$1"
        shift
        ;;
esac

_nvr_check_prereqs || exit 1

cd "$_nvr_root"

# Default to the whole workspace unless the caller picked a package.
scope=(--workspace)
for arg in "$@"; do
    case "$arg" in
        -p | --package | -p* | --package=*) scope=() ;;
    esac
done

case "$mode" in
    check) set -x; exec cargo check "${scope[@]}" "$@" ;;
    build) set -x; exec cargo build "${scope[@]}" "$@" ;;
    release) set -x; exec cargo build "${scope[@]}" --release "$@" ;;
esac
