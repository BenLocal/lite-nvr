#!/bin/sh
set -eu
root=$1
out=$2
card=$3
connector=$4
export LD_LIBRARY_PATH="$root/lib:$root/ffmpeg/lib"
case "$5" in
    conditions)
        cat "/sys/class/drm/$(basename "$card")-$connector/status"
        wc -c < "/sys/class/drm/$(basename "$card")-$connector/edid"
        wc -l < "/sys/class/drm/$(basename "$card")-$connector/modes"
        ;;
    list) "$out/hdmiout" list "$card" ;;
    pattern) "$out/hdmiout" pattern "$card" "$connector" "${6:-20}" ;;
    dual) "$out/dual_hdmiout" 5 ;;
    grab)
        "$root/ffmpeg/bin/ffmpeg" -nostdin -hide_banner -f v4l2 \
            -i /dev/video11 -frames:v 50 -c:v libx264 -y "$out/loop.mp4"
        ;;
    *) exit 2 ;;
esac
