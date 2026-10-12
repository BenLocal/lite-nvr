#!/bin/sh
set -eu
root=$1
out=$2
export LD_LIBRARY_PATH="$root/lib:$root/ffmpeg/lib"
ffmpeg="$root/ffmpeg/bin/ffmpeg"
case "$3" in
    encode)
        "$ffmpeg" -nostdin -hide_banner -loglevel verbose -f lavfi \
            -i testsrc2=size=1280x720:rate=25 -frames:v 50 -pix_fmt nv12 \
            -c:v h264_rkmpp -y "$out/hardware.mp4"
        test -s "$out/hardware.mp4"
        "$ffmpeg" -nostdin -v error -i "$out/hardware.mp4" -f null -
        ;;
    decode)
        "$ffmpeg" -nostdin -hide_banner -loglevel verbose -c:v h264_rkmpp \
            -i "$out/hardware.mp4" -frames:v 50 -f null -
        ;;
    *) exit 2 ;;
esac
