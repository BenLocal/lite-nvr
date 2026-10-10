#!/bin/sh
# lite-nvr launcher for the Rockchip package (POSIX sh: works with BusyBox ash).
#
#   ./start.sh [run]   foreground, logs to the terminal (default; used by the Docker setup)
#   ./start.sh start   background, logs to nvr.log (the previous log becomes nvr.log.prev)
#   ./start.sh stop    stop the nvr started from this directory
#   ./start.sh restart stop, then start
#   ./start.sh status  process, listening ports and admin page; exit 3 when not running
#
# Environment (e.g. RUST_LOG, NVR_RECORD_DIR, FFMPEG_BUS_ENCODER_QUEUE_FRAMES) is
# passed through to nvr. Data (nvr.db, data/records) lives in this directory.
set -eu

root=$(cd "$(dirname "$0")" && pwd)
bin="$root/bin/nvr"
log="$root/nvr.log"
ports="18080 8553 ${NVR_ZLM_RTSP_PORT:-8554} 8555"

cd "$root"
export LD_LIBRARY_PATH="$root/lib:$root/ffmpeg/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$root/ffmpeg/bin:$PATH"
export ORT_DYLIB_PATH="${ORT_DYLIB_PATH:-$root/lib/libonnxruntime.so}"
export RUST_LOG="${RUST_LOG:-info}"

# PIDs of nvr processes running this package's binary. Matched by executable
# path, so other services on the board are never touched.
nvr_pids() {
    for p in /proc/[0-9]*; do
        case "$(readlink "$p/exe" 2>/dev/null)" in
            "$bin" | "$bin (deleted)") echo "${p#/proc/}" ;;
        esac
    done
}

start() {
    running=$(nvr_pids)
    if [ -n "$running" ]; then
        echo "nvr already running (pid $(echo $running))"
        return 0
    fi
    [ -f "$log" ] && mv "$log" "$log.prev"
    nohup "$bin" "$@" > "$log" 2>&1 < /dev/null &
    sleep 2
    running=$(nvr_pids)
    if [ -z "$running" ]; then
        echo "nvr exited right after start; last log lines:" >&2
        tail -n 20 "$log" >&2
        return 1
    fi
    echo "nvr started (pid $(echo $running)), log: $log"
}

stop() {
    running=$(nvr_pids)
    if [ -z "$running" ]; then
        echo "nvr not running"
        return 0
    fi
    kill $running
    waited=0
    while [ -n "$(nvr_pids)" ] && [ "$waited" -lt 15 ]; do
        sleep 1
        waited=$((waited + 1))
    done
    left=$(nvr_pids)
    if [ -n "$left" ]; then
        echo "nvr still running after ${waited}s, killing pid $(echo $left)"
        kill -9 $left
    fi
    echo "nvr stopped"
}

status() {
    running=$(nvr_pids)
    if [ -z "$running" ]; then
        echo "nvr not running"
        return 3
    fi
    echo "nvr running (pid $(echo $running))"
    for port in $ports; do
        if netstat -ltn 2>/dev/null | grep -q ":$port "; then
            echo "  port $port listening"
        else
            echo "  port $port NOT listening"
        fi
    done
    if command -v wget > /dev/null 2>&1; then
        if wget -q -O /dev/null "http://127.0.0.1:18080/nvr/"; then
            echo "  admin page http://<board>:18080/nvr/ OK"
        else
            echo "  admin page not responding"
        fi
    fi
}

cmd=${1:-run}
[ "$#" -gt 0 ] && shift
case "$cmd" in
    run) exec "$bin" "$@" ;;
    start) start "$@" ;;
    stop) stop ;;
    restart)
        stop
        start "$@"
        ;;
    status) status ;;
    *)
        echo "usage: $0 [run|start|stop|restart|status]" >&2
        exit 2
        ;;
esac
