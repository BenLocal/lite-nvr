#!/usr/bin/env bash
# Check the package in a base OS container with networking disabled.
set -euo pipefail
package="$(cd "$1" && pwd)"
image="${PACKAGE_TEST_IMAGE:-ubuntu:22.04}"
docker run --rm --network none --cpus=2 --memory=2g \
    -v "$package:/input:ro" "$image" bash -euo pipefail -c '
    mkdir -p /work/bin
    cp /input/start.sh /work/start.sh
    ln -s /input/bin/nvr /work/bin/nvr
    ln -s /input/lib /work/lib
    ln -s /input/ffmpeg /work/ffmpeg
    export LD_LIBRARY_PATH=/input/lib:/input/ffmpeg/lib
    export PATH=/input/bin:/input/ffmpeg/bin:$PATH
    for file in /input/bin/* /input/lib/*.so* /input/ffmpeg/bin/* /input/ffmpeg/lib/*.so*; do
        output=$(ldd "$file" 2>&1) || {
            case "$output" in *"not a dynamic executable"*|*"statically linked"*) continue ;; esac
            printf "%s\n" "$output" >&2; exit 1
        }
        if [[ "$output" == *"not found"* ]]; then printf "%s\n" "$output" >&2; exit 1; fi
    done
    ffmpeg -version >/dev/null
    ffprobe -version >/dev/null
    /work/start.sh >/tmp/nvr.log 2>&1 &
    server=$!
    trap "kill $server 2>/dev/null || true" EXIT
    for attempt in {1..30}; do
        if ! kill -0 "$server" 2>/dev/null; then cat /tmp/nvr.log >&2; exit 1; fi
        if { exec 3<>/dev/tcp/127.0.0.1/18080; } 2>/dev/null; then
            printf "GET /nvr/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n" >&3
            IFS= read -r status <&3
            exec 3>&-
            if [[ "$status" == *" 200 "* ]]; then
                echo "Offline startup and embedded dashboard passed."
                exit 0
            fi
        fi
        sleep 1
    done
    cat /tmp/nvr.log >&2
    exit 1
'
