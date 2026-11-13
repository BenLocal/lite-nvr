#!/bin/sh
set -eu
root=$1
out=$2
port=$3
export LD_LIBRARY_PATH="$root/lib:$root/ffmpeg/lib"
ffmpeg="$root/ffmpeg/bin/ffmpeg"
push() {
    nohup "$ffmpeg" -nostdin -hide_banner -v warning -re -stream_loop -1 \
        -i "$out/source.mp4" -c copy -f rtsp -rtsp_transport tcp \
        "rtsp://127.0.0.1:$port/e2e/$1" > "$out/source.log" 2>&1 < /dev/null &
    echo "$!" > "$out/source.pid"
}
case "$4" in
    start)
        "$ffmpeg" -nostdin -hide_banner -v error -f lavfi -i testsrc2=size=640x360:rate=25 \
            -f lavfi -i sine=frequency=1000:sample_rate=48000 -t 3 \
            -c:v h264_rkmpp -pix_fmt nv12 -c:a aac -y "$out/source.mp4"
        push source
        ;;
    speech)
        # Speech sample plus 2 s of silence so VAD closes a segment on every loop.
        mkdir -p "$out"
        "$ffmpeg" -nostdin -hide_banner -v error -f lavfi -i testsrc2=size=640x360:rate=25 \
            -i "$5" -af apad=pad_dur=2 -shortest \
            -c:v h264_rkmpp -pix_fmt nv12 -c:a aac -ar 48000 -y "$out/source.mp4"
        push speech
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
