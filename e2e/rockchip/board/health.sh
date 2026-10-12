#!/bin/sh
set -eu
root=$1
export LD_LIBRARY_PATH="$root/lib:$root/ffmpeg/lib"
case "$2" in
    status)
        cd "$root"
        ./start.sh status
        for p in /proc/[0-9]*; do
            if [ "$(readlink "$p/exe" 2>/dev/null)" = "$root/bin/nvr" ]; then
                echo "pid=${p#/proc/}"
            fi
        done
        cat /proc/uptime
        cat /proc/sys/kernel/random/boot_id
        ;;
    libs)
        ldd "$root/bin/nvr"
        ldd "$root/ffmpeg/lib/libavcodec.so"
        ;;
    libc) /lib/libc.so.6 ;;
    codecs)
        "$root/ffmpeg/bin/ffmpeg" -hide_banner -encoders 2>&1
        "$root/ffmpeg/bin/ffmpeg" -hide_banner -decoders 2>&1
        ;;
    resources)
        for p in /proc/[0-9]*; do
            if [ "$(readlink "$p/exe" 2>/dev/null)" = "$root/bin/nvr" ]; then
                grep -E 'VmRSS|VmHWM|Threads' "$p/status"
            fi
        done
        cat /proc/loadavg
        cat /proc/meminfo
        ;;
    *) exit 2 ;;
esac
