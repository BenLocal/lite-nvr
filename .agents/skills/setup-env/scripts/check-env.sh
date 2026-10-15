#!/usr/bin/env bash
# Read-only snapshot of the local lite-nvr dev environment.
# Prints one line per item: OK / MISSING / INFO <item> <detail>. Changes nothing.
set -uo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$root" || exit 1

ok() { printf 'OK      %-22s %s\n' "$1" "${2:-}"; }
missing() { printf 'MISSING %-22s %s\n' "$1" "${2:-}"; }
info() { printf 'INFO    %-22s %s\n' "$1" "${2:-}"; }

tool() {
    if command -v "$1" >/dev/null 2>&1; then
        ok "$1" "$(timeout 5 "$1" --version 2>&1 | head -n1)"
    else
        missing "$1" "${2:-}"
    fi
}

# Read a key from .env without sourcing it (values may hold secrets).
env_key() { grep -E "^$1=" .env 2>/dev/null | tail -n1 | cut -d= -f2-; }

echo "== host"
info platform "$(uname -s) $(uname -m)"

echo "== toolchain"
tool cargo
tool rustup
tool node
tool npm
tool pkg-config
tool clang
# `cross --version` probes the container engine and can hang; only check presence.
if command -v cross >/dev/null 2>&1; then ok cross "$(command -v cross)"; else missing cross "only for make package"; fi
tool docker "only for make package"

echo "== .env"
if [ -f .env ]; then
    ok .env
    for k in HTTP_PROXY HTTPS_PROXY NO_PROXY FFMPEG_DIR ZLM_DIR RK_FFMPEG_URL APT_MIRROR \
        ASR_MODELS_DIR DETECT_MODELS_DIR RTSP_TEST_URL ONVIF_TEST_HOST FTP_TEST_ROOT XIAOMI_USER; do
        v="$(env_key "$k")"
        case "$k" in
            XIAOMI_USER | ONVIF_TEST_HOST) [ -n "$v" ] && v="<set>" ;;
        esac
        [ -n "$v" ] && info "$k" "$v"
    done
    proxy="$(env_key HTTPS_PROXY)"
    case "$proxy" in socks*) missing HTTPS_PROXY "is SOCKS; build downloads need an HTTP proxy" ;; esac
else
    missing .env "cp .env.example .env"
fi

echo "== native deps"
ffmpeg_dir="$(env_key FFMPEG_DIR)"
ffmpeg_dir="${ffmpeg_dir:-$root/ffmpeg}"
if [ -f "$ffmpeg_dir/include/libavcodec/avcodec.h" ]; then
    ok ffmpeg "$ffmpeg_dir"
elif pkg-config --exists libavcodec 2>/dev/null; then
    ok ffmpeg "system libavcodec $(pkg-config --modversion libavcodec)"
else
    missing ffmpeg "make install-deps"
fi
zlm_dir="$(env_key ZLM_DIR)"
zlm_dir="${zlm_dir:-$root/zlm}"
if [ -d "$zlm_dir/lib" ]; then ok zlm "$zlm_dir"; else missing zlm "make install-deps (else rszlm-sys downloads it, ignoring the proxy)"; fi
if [ -n "$(ls -d third_party/sherpa-onnx/*/lib 2>/dev/null)" ]; then ok sherpa-onnx-libs; else missing sherpa-onnx-libs "optional: make download-asr-libs"; fi

echo "== models"
asr_dir="$(env_key ASR_MODELS_DIR)"
asr_dir="${asr_dir:-third_party/asr-models}"
if [ -d "$asr_dir" ]; then ok asr-models "$asr_dir"; else missing asr-models "optional: make download-asr-models"; fi
det_dir="$(env_key DETECT_MODELS_DIR)"
det_dir="${det_dir:-third_party/detect-models}"
if [ -f "$det_dir/models.json" ]; then ok detect-models "$det_dir/models.json"; else missing detect-models "optional: $det_dir/models.json"; fi

echo "== frontend"
if [ -d nvr-dashboard/app/node_modules ]; then ok node_modules; else missing node_modules "optional: make frontend-install (build.rs runs npm ci anyway)"; fi

echo "== rockchip"
if [ -f target/cross-deps/rockchip/ffmpeg/include/libavcodec/avcodec.h ]; then
    ok rk-ffmpeg-sdk target/cross-deps/rockchip/ffmpeg
else
    info rk-ffmpeg-sdk "not downloaded (fetched on first rockchip build)"
fi
if [ -f dev-env.local.md ]; then ok dev-env.local.md "test host notes present"; else info dev-env.local.md "absent"; fi
