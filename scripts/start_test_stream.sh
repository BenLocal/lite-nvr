#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Source proxy settings from .env
if [ -f "$PROJECT_DIR/.env" ]; then
    set -a; . "$PROJECT_DIR/.env"; set +a
fi

# --------------- defaults ---------------
PORT="${PORT:-9554}"
STREAM_PATH="${STREAM_PATH:-/live/test1}"
WIDTH="${WIDTH:-1920}"
HEIGHT="${HEIGHT:-1080}"
FPS="${FPS:-25}"
PRESET="${PRESET:-ultrafast}"
INSTANCE="${INSTANCE:-1}"
# ------------------------------------------

if [ "$INSTANCE" -gt 1 ]; then
    STREAM_PATH="/live/test$INSTANCE"
    PORT=$((PORT + INSTANCE - 1))
fi

cd "$PROJECT_DIR"

echo "=== dummy-rtsp-camera ==="
echo "  rtsp url  : rtsp://127.0.0.1:$PORT$STREAM_PATH"
echo "  size      : ${WIDTH}x${HEIGHT}"
echo "  fps       : $FPS"
echo "  preset    : $PRESET"
echo ""

exec cargo run -p dummy-rtsp-camera -- \
    --port "$PORT" \
    --path "$STREAM_PATH" \
    --width "$WIDTH" \
    --height "$HEIGHT" \
    --fps "$FPS" \
    --preset "$PRESET"
