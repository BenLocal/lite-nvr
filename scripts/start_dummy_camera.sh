#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# --------------- defaults ---------------
SERVER_ADDR="${SERVER_ADDR:-127.0.0.1:5060}"
SERVER_ID="${SERVER_ID:-34020000002000000001}"
DEVICE_ID="${DEVICE_ID:-34020000001320000001}"
CHANNEL_ID="${CHANNEL_ID:-$DEVICE_ID}"
PASSWORD="${PASSWORD:-12345678}"
DOMAIN="${DOMAIN:-${SERVER_ID:0:10}}"
MEDIA_IP="${MEDIA_IP:-127.0.0.1}"
LISTEN="${LISTEN:-0.0.0.0:5061}"
FPS="${FPS:-25}"
RUST_LOG="${RUST_LOG:-info}"
SOURCE_FILE="${SOURCE_FILE:-}"
MANUFACTURER="${MANUFACTURER:-lite-nvr}"
MODEL="${MODEL:-dummy-camera}"
FIRMWARE="${FIRMWARE:-0.1}"
INSTANCE="${INSTANCE:-1}"
# ------------------------------------------

# when INSTANCE > 1, auto-offset ports and IDs so they don't clash
if [ "$INSTANCE" -gt 1 ]; then
    OFFSET=$((INSTANCE - 1))

    # bump SIP server port
    IFS=':' read -r SA_HOST SA_PORT <<<"$SERVER_ADDR"
    SA_PORT=$((SA_PORT + OFFSET))
    SERVER_ADDR="${SA_HOST}:${SA_PORT}"

    # bump device-id (increment the numeric suffix of the 20-digit code)
    DEVICE_ID_NUM=$(echo "$DEVICE_ID" | sed 's/^0*//')
    DEVICE_ID_NUM=$((DEVICE_ID_NUM + OFFSET))
    DEVICE_ID=$(printf "%020d" "$DEVICE_ID_NUM")

    # bump channel-id similarly
    CHANNEL_ID_NUM=$(echo "$CHANNEL_ID" | sed 's/^0*//')
    CHANNEL_ID_NUM=$((CHANNEL_ID_NUM + OFFSET))
    CHANNEL_ID=$(printf "%020d" "$CHANNEL_ID_NUM")

    # bump listen port
    LISTEN_PREFIX="${LISTEN%:*}"
    LISTEN_PORT="${LISTEN##*:}"
    LISTEN_PORT=$((LISTEN_PORT + OFFSET))
    LISTEN="${LISTEN_PREFIX}:${LISTEN_PORT}"
fi

cd "$PROJECT_DIR"

ARGS=(
    --server-addr "$SERVER_ADDR"
    --server-id   "$SERVER_ID"
    --device-id   "$DEVICE_ID"
    --channel-id  "$CHANNEL_ID"
    --password    "$PASSWORD"
    --domain      "$DOMAIN"
    --media-ip    "$MEDIA_IP"
    --listen      "$LISTEN"
    --fps         "$FPS"
    --manufacturer "$MANUFACTURER"
    --model       "$MODEL"
    --firmware    "$FIRMWARE"
)

if [ -n "$SOURCE_FILE" ]; then
    ARGS+=(--source-file "$SOURCE_FILE")
fi

echo "=== dummy-camera instance $INSTANCE ==="
echo "  server-addr  : $SERVER_ADDR"
echo "  server-id    : $SERVER_ID"
echo "  device-id    : $DEVICE_ID"
echo "  channel-id   : $CHANNEL_ID"
echo "  listen       : $LISTEN"
echo "  media-ip     : $MEDIA_IP"
echo "  source-file  : ${SOURCE_FILE:-<bundled clip>}"
echo ""

export RUST_LOG
exec cargo run -p dummy-camera -- "${ARGS[@]}"
