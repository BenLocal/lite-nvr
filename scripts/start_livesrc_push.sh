#!/usr/bin/env bash
set -euo pipefail

# Pushes a looping H.264 test pattern INTO the running NVR's ZLM over RTSP,
# publishing it as `live/livesrc` — playable at /media/live/livesrc.live.flv
# and usable as a switcher/compositor source. The NVR must be running first
# (ZLM RTSP server on :8554).

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Source proxy settings from .env
if [ -f "$PROJECT_DIR/.env" ]; then
    set -a; . "$PROJECT_DIR/.env"; set +a
fi

# --------------- defaults ---------------
HOST="${HOST:-127.0.0.1}"
PORT="${PORT:-8554}"            # ZLM RTSP port
APP="${APP:-live}"
STREAM="${STREAM:-livesrc}"
WIDTH="${WIDTH:-1920}"
HEIGHT="${HEIGHT:-1080}"
FPS="${FPS:-25}"
PRESET="${PRESET:-ultrafast}"
# -----------------------------------------

# Prefer the bundled ffmpeg (needs its own lib dir on LD_LIBRARY_PATH or it
# fails with exit 127), fall back to ffmpeg from PATH.
FFMPEG_DIR="${FFMPEG_DIR:-$PROJECT_DIR/ffmpeg}"
if [ -x "$FFMPEG_DIR/bin/ffmpeg" ]; then
    FFMPEG="$FFMPEG_DIR/bin/ffmpeg"
    export LD_LIBRARY_PATH="$FFMPEG_DIR/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
else
    FFMPEG="ffmpeg"
fi

URL="rtsp://$HOST:$PORT/$APP/$STREAM"

echo "=== livesrc push (test pattern -> ZLM) ==="
echo "  push url : $URL"
echo "  play flv : /media/$APP/$STREAM.live.flv (dashboard) or http://$HOST:8553/$APP/$STREAM.live.flv"
echo "  size     : ${WIDTH}x${HEIGHT} @ ${FPS}fps ($PRESET)"
echo ""

exec "$FFMPEG" -hide_banner -re \
    -f lavfi -i "testsrc2=size=${WIDTH}x${HEIGHT}:rate=${FPS}" \
    -c:v libx264 -preset "$PRESET" -tune zerolatency -pix_fmt yuv420p -g "$((FPS * 2))" \
    -f rtsp -rtsp_transport tcp "$URL"
