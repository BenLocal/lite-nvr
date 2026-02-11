#!/usr/bin/env bash
set -euo pipefail

# 结束占用 :99 的旧 Xvfb，避免 "server already running"
pkill -f "Xvfb :99" 2>/dev/null || true
sleep 2

rm -rf /tmp/.X99-lock /tmp/.X11-unix/X99
Xvfb :99 -screen 0 1920x1080x24 &
# 等 Xvfb 就绪再连，避免 "Can't open display: :99"
for i in $(seq 1 10); do
  [ -S /tmp/.X11-unix/X99 ] && break
  sleep 1
done
DISPLAY=:99 xsetroot -solid gray &

# 在 xterm 窗口中显示从 0 开始的秒表（HH:MM:SS）
DISPLAY=:99 xterm -geometry 50x8 -e 'bash -c "start=\$(date +%s); while true; do now=\$(date +%s); elapsed=\$((now-start)); printf \"\r  Stopwatch: %02d:%02d:%02d  \" \$((elapsed/3600)) \$(((elapsed/60)%60)) \$((elapsed%60)); sleep 1; done"' &

# ffmpeg
# ./ffmpeg -f x11grab -framerate 25 -video_size 1920x1080 -i :99  \
# -c:v libx264 -preset veryfast -b:v 2000k -maxrate 2000k -bufsize 1000k  \
# -pix_fmt yuv420p -g 50 -keyint_min 25  \
# -f rtsp -rtsp_transport tcp rtsp://127.0.0.1:8554/live/screen