#!/bin/sh
set -eu
root=$1
out=$2
port=$3
export LD_LIBRARY_PATH="$root/lib:$root/ffmpeg/lib"
ffmpeg="$root/ffmpeg/bin/ffmpeg"
case "$4" in
    start)
        "$ffmpeg" -nostdin -hide_banner -v error -f lavfi -i testsrc2=size=640x360:rate=25 \
            -f lavfi -i sine=frequency=1000:sample_rate=48000 -t 3 \
            -c:v h264_rkmpp -pix_fmt nv12 -c:a aac -y "$out/source.mp4"
        nohup "$ffmpeg" -nostdin -hide_banner -v warning -re -stream_loop -1 \
            -i "$out/source.mp4" -c copy -f rtsp -rtsp_transport tcp \
            "rtsp://127.0.0.1:$port/e2e/source" > "$out/source.log" 2>&1 < /dev/null &
        echo "$!" > "$out/source.pid"
        ;;
    stop)
        if [ -f "$out/source.pid" ]; then
            pid=$(cat "$out/source.pid")
            case "$pid" in ''|*[!0-9]*) exit 1 ;; esac
            if [ "$(readlink "/proc/$pid/exe" 2>/dev/null)" = "$ffmpeg" ]; then
                if tr '\000' ' ' < "/proc/$pid/cmdline" | grep -F -q "$out/source.mp4"; then
                    kill "$pid"
                fi
            fi
        fi
        ;;
    *) exit 2 ;;
esac
