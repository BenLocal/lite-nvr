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
    dual) "$out/dual_hdmiout" 5 "$card" "$connector" "$6" ;;
    sample)
        node=$6
        fps=$7
        "$root/ffmpeg/bin/ffmpeg" -nostdin -hide_banner -f v4l2 -list_formats all \
            -i "$node" > "$out/capture-formats.log" 2>&1 || true
        if grep -q 'nv12' "$out/capture-formats.log"; then
            set -- -input_format nv12
        else
            set --
        fi
        if ! "$root/ffmpeg/bin/ffmpeg" -nostdin -hide_banner -f v4l2 "$@" -framerate "$fps" \
            -i "$node" -vf 'crop=iw:ih/2:0:0,format=rgb24,scale=8:1' -frames:v 1 -vsync 0 \
            -pix_fmt rgb24 -f rawvideo -y "$out/signal.rgb" > "$out/capture-signal.log" 2>&1; then
            cat "$out/capture-signal.log"
            exit 1
        fi
        test "$(wc -c < "$out/signal.rgb")" -eq 24
        od -An -tu1 "$out/signal.rgb"
        ;;
    *) exit 2 ;;
esac
