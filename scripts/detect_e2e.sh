#!/usr/bin/env bash
#
# detect_e2e.sh — one-shot end-to-end demo of nvr's real-time multi-model
# object detection.
#
# It downloads a matching ONNX Runtime shared library + two usls-native YOLO
# models (yolov8n, yolo11n), starts a dummy RTSP camera looping a bus image and
# an nvr instance, then runs BOTH models on the same frames and prints the
# side-by-side comparison. Everything is torn down on exit.
#
# All downloaded artifacts land in third_party/ (git-ignored). Re-runs reuse
# cached downloads. Requires: curl, python3, the project's bundled ffmpeg, and a
# built (or buildable) nvr. Network access is needed only for the first run.
#
# Usage:  bash scripts/detect_e2e.sh
#
set -euo pipefail

# --- locate repo root ---------------------------------------------------------
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# --- config -------------------------------------------------------------------
# ONNX Runtime version MUST match ort-sys' ONNXRUNTIME_VERSION for the pinned
# `ort =2.0.0-rc.10` (usls dependency). If you bump usls/ort, update this.
ORT_VER="1.22.0"
ORT_TARBALL="onnxruntime-linux-x64-${ORT_VER}.tgz"
ORT_URL="https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VER}/${ORT_TARBALL}"

# usls's default model hub is github.com/jamjamjon/assets; these are its native
# YOLO detect exports (guaranteed to parse with usls' YOLO backend).
YOLOV8_URL="https://github.com/jamjamjon/assets/releases/download/yolo/v8-n-det.onnx"
YOLO11_URL="https://github.com/jamjamjon/assets/releases/download/yolo/v11-n-det.onnx"
BUS_URL="https://ultralytics.com/images/bus.jpg"

MODELS_DIR="$ROOT/third_party/detect-models"
ORT_DIR="$ROOT/third_party/onnxruntime"
ORT_SO="$ORT_DIR/onnxruntime-linux-x64-${ORT_VER}/lib/libonnxruntime.so"

API="http://127.0.0.1:18080"
API_PORT="18080"
RTSP_PORT="9554"
STREAM_PATH="/live/test1"
DEVICE_ID="det-e2e"
FFMPEG="$ROOT/ffmpeg/bin/ffmpeg"

log()  { printf '\n\033[1;36m== %s ==\033[0m\n' "$*"; }
info() { printf '   %s\n' "$*"; }
die()  { printf '\033[1;31mERROR: %s\033[0m\n' "$*" >&2; exit 1; }

# --- cleanup trap -------------------------------------------------------------
TOKEN=""; RTSP_PID=""; NVR_PID=""
cleanup() {
  log "cleanup"
  if [ -n "$TOKEN" ]; then
    curl -s -X POST "$API/api/detect/$DEVICE_ID/stop?token=$TOKEN" >/dev/null 2>&1 || true
    curl -s -X POST "$API/api/device/del?token=$TOKEN" \
      -H 'content-type: application/json' -d "{\"id\":\"$DEVICE_ID\"}" >/dev/null 2>&1 || true
  fi
  [ -n "$NVR_PID" ]  && kill -9 "$NVR_PID"  2>/dev/null || true
  [ -n "$RTSP_PID" ] && kill -9 "$RTSP_PID" 2>/dev/null || true
  # sweep anything still bound to the ports we own (e.g. the oddity child of the
  # dummy-rtsp cargo wrapper). Safe because we bailed at startup if the ports
  # were already in use by someone else.
  local pids
  pids="$(ss -ltnp 2>/dev/null | grep -E ":${API_PORT}|:${RTSP_PORT}|:8553|:8554" \
          | grep -oE 'pid=[0-9]+' | grep -oE '[0-9]+' | sort -u || true)"
  for p in $pids; do kill -9 "$p" 2>/dev/null || true; done
  info "services stopped, ports released"
}
trap cleanup EXIT

# --- helpers ------------------------------------------------------------------
port_in_use() { ss -ltn 2>/dev/null | grep -q ":$1 "; }

# Download $2 -> $1 unless it already exists and is non-trivial in size.
fetch() {
  local dst="$1" url="$2" min="${3:-100000}"
  if [ -f "$dst" ] && [ "$(stat -c%s "$dst")" -ge "$min" ]; then
    info "have $(basename "$dst") ($(stat -c%s "$dst") bytes)"; return 0
  fi
  info "downloading $(basename "$dst") ..."
  curl -fsSL --retry 3 --max-time 600 -o "$dst" "$url" \
    || die "download failed: $url"
  [ "$(stat -c%s "$dst")" -ge "$min" ] || die "downloaded $(basename "$dst") too small"
}

# Wait until $1 is listening (or nvr died / timeout).
wait_port() {
  local port="$1" what="$2" deadline=$(( $(date +%s) + 120 ))
  while :; do
    port_in_use "$port" && { info "$what up on :$port"; return 0; }
    [ -n "$NVR_PID" ] && ! kill -0 "$NVR_PID" 2>/dev/null && die "$what died (see nvr.log)"
    [ "$(date +%s)" -ge "$deadline" ] && die "timed out waiting for $what on :$port"
    sleep 3
  done
}

# --- 0. preconditions ---------------------------------------------------------
log "preconditions"
command -v curl   >/dev/null || die "curl not found"
command -v python3 >/dev/null || die "python3 not found"
[ -x "$FFMPEG" ]  || die "bundled ffmpeg not found at $FFMPEG (run scripts/pre_install_deps.sh)"
if port_in_use "$API_PORT"; then die "port $API_PORT already in use — stop the running nvr first"; fi
if port_in_use "$RTSP_PORT"; then die "port $RTSP_PORT already in use — stop whatever holds it first"; fi
mkdir -p "$MODELS_DIR" "$ORT_DIR"

# --- 1. artifacts -------------------------------------------------------------
log "artifacts (ONNX Runtime + models + test clip)"
if [ ! -f "$ORT_SO" ]; then
  fetch "$ORT_DIR/$ORT_TARBALL" "$ORT_URL" 3000000
  tar -xzf "$ORT_DIR/$ORT_TARBALL" -C "$ORT_DIR"
fi
[ -f "$ORT_SO" ] || die "libonnxruntime.so missing after extract"
info "ONNX Runtime: $ORT_SO"
fetch "$MODELS_DIR/yolov8n.onnx" "$YOLOV8_URL" 5000000
fetch "$MODELS_DIR/yolo11n.onnx" "$YOLO11_URL" 5000000
fetch "$MODELS_DIR/bus.jpg"      "$BUS_URL"    50000
if [ ! -f "$MODELS_DIR/bus.mp4" ]; then
  info "rendering looping bus.mp4 from bus.jpg ..."
  LD_LIBRARY_PATH="$ROOT/ffmpeg/lib" "$FFMPEG" -y -loglevel error \
    -loop 1 -i "$MODELS_DIR/bus.jpg" -c:v libx264 -t 10 -r 15 \
    -pix_fmt yuv420p -vf "scale=810:1080" "$MODELS_DIR/bus.mp4" \
    || die "ffmpeg failed to render bus.mp4"
fi

# models.json — every model MUST carry `version` (usls bails otherwise).
cat > "$MODELS_DIR/models.json" <<'JSON'
[
  { "name": "yolov8n", "model_file": "yolov8n.onnx", "version": 8.0,  "conf": 0.25 },
  { "name": "yolo11n", "model_file": "yolo11n.onnx", "version": 11.0, "conf": 0.25 }
]
JSON
info "models.json -> yolov8n + yolo11n"

# --- 2. build nvr -------------------------------------------------------------
# Do NOT export ZLM_DIR: setting it retriggers the rszlm-sys build script, which
# needs zlm/include headers that a runtime-only checkout may lack. The default
# resolution used by a normal `cargo build` works.
log "build nvr"
LD_LIBRARY_PATH="$ROOT/ffmpeg/lib:$ROOT/zlm/lib:$ROOT/target/debug/deps" \
FFMPEG_DIR="$ROOT/ffmpeg" \
  cargo build -q -p nvr || die "cargo build -p nvr failed"
[ -x "$ROOT/target/debug/nvr" ] || die "target/debug/nvr missing after build"

# --- 3. start services --------------------------------------------------------
SCRATCH="$(mktemp -d)"
log "start dummy RTSP camera (looping bus.mp4)"
LD_LIBRARY_PATH="$ROOT/ffmpeg/lib:$ROOT/zlm/lib:$ROOT/target/debug/deps" \
FFMPEG_DIR="$ROOT/ffmpeg" \
  cargo run -q -p dummy-rtsp-camera -- \
    --media "$MODELS_DIR/bus.mp4" --port "$RTSP_PORT" --path "$STREAM_PATH" \
    > "$SCRATCH/rtsp.log" 2>&1 &
RTSP_PID=$!
wait_port "$RTSP_PORT" "rtsp"

log "start nvr (ORT load-dynamic; ORT_DYLIB_PATH -> our libonnxruntime.so)"
LD_LIBRARY_PATH="$ROOT/ffmpeg/lib:$ROOT/target/debug/deps" \
FFMPEG_DIR="$ROOT/ffmpeg" \
ORT_DYLIB_PATH="$ORT_SO" \
DETECT_MODELS_DIR="$MODELS_DIR" \
RUST_LOG="info,nvr::detect=debug" \
  "$ROOT/target/debug/nvr" > "$SCRATCH/nvr.log" 2>&1 &
NVR_PID=$!
wait_port "$API_PORT" "nvr api"

# --- 4. drive the API ---------------------------------------------------------
log "login + add device + start detection"
TOKEN="$(curl -s -X POST "$API/api/user/login" -H 'content-type: application/json' \
  -d '{"username":"admin","password":"admin"}' \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["token"])')"
[ -n "$TOKEN" ] || die "login failed"
info "models configured: $(curl -s "$API/api/detect/models?token=$TOKEN")"
curl -s -X POST "$API/api/device/add?token=$TOKEN" -H 'content-type: application/json' \
  -d "{\"id\":\"$DEVICE_ID\",\"name\":\"detect e2e\",\"input_type\":\"rtsp\",\"input_value\":\"rtsp://127.0.0.1:$RTSP_PORT$STREAM_PATH\"}" \
  >/dev/null || die "device add failed"
info "device $DEVICE_ID added -> rtsp://127.0.0.1:$RTSP_PORT$STREAM_PATH"
# small settle so the pipe is pulling before the first detect sample
sleep 4
curl -s -X POST "$API/api/detect/$DEVICE_ID/start?token=$TOKEN" \
  -H 'content-type: application/json' -d '{"models":["yolov8n","yolo11n"]}' >/dev/null \
  || die "detect start failed"
info "detection started (yolov8n + yolo11n)"

# --- 5. poll + print the same-frame comparison --------------------------------
cat > "$SCRATCH/cmp.py" <<'PY'
import sys, json, collections
try:
    d = json.load(sys.stdin)
except Exception:
    print("   (no result yet)"); sys.exit(1)
ms = d.get("models", [])
tot = sum(len(m.get("detections", [])) for m in ms)
print("   SAME FRAME %sx%s  ts=%s  (%d models)"
      % (d.get("frame_w"), d.get("frame_h"), d.get("ts"), len(ms)))
for m in ms:
    c = collections.Counter(x["label"] for x in m["detections"])
    summ = ", ".join("%dx %s" % (n, l) for l, n in c.most_common())
    print("     %-8s %2d boxes %7.1fms | %s"
          % (m["name"], len(m["detections"]), m["infer_ms"], summ or "-"))
sys.exit(0 if (len(ms) >= 2 and tot > 0) else 1)
PY

log "same-frame comparison (first call builds both models; ~2fps thereafter)"
deadline=$(( $(date +%s) + 60 )); ok=1
while :; do
  if curl -s "$API/api/detect/$DEVICE_ID/latest?token=$TOKEN" | python3 "$SCRATCH/cmp.py"; then
    ok=0; break
  fi
  ! kill -0 "$NVR_PID" 2>/dev/null && die "nvr crashed during detection (see $SCRATCH/nvr.log)"
  [ "$(date +%s)" -ge "$deadline" ] && break
  sleep 4
done
[ "$ok" -eq 0 ] || die "no detections within timeout (see $SCRATCH/nvr.log)"

log "a few steady-state samples"
for _ in 1 2 3; do
  curl -s "$API/api/detect/$DEVICE_ID/latest?token=$TOKEN" | python3 "$SCRATCH/cmp.py" || true
  sleep 2
done

log "SUCCESS — real-time multi-model detection verified end-to-end"
# cleanup runs via the EXIT trap
