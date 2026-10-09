#!/usr/bin/env bash
# Fetch pristine pinned crate sources and apply the RK patch once per digest.
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ -f .env ]]; then
    set -a
    source .env
    set +a
fi
export https_proxy="${https_proxy:-${HTTPS_PROXY:-}}"
export http_proxy="${http_proxy:-${HTTP_PROXY:-}}"
patch_file=patches/ffmpeg-rockchip-8.1.0.patch
digest=$(shasum -a 256 "$patch_file" | awk '{print $1}')
cache=".cache/rockchip-rust/$digest"
if [[ ! -f "$cache/.ready" ]]; then
    mkdir -p "$cache"
    for crate in ffmpeg-next ffmpeg-sys-next; do
        source_dir=""
        for candidate in "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/"$crate-8.1.0"; do
            if [[ -d "$candidate" ]]; then source_dir="$candidate"; break; fi
        done
        mkdir -p "$cache/$crate"
        if [[ -n "$source_dir" ]]; then
            cp -R "$source_dir/." "$cache/$crate/"
        else
            curl -fL --retry 3 -o "$cache/$crate.crate" \
                "https://static.crates.io/crates/$crate/$crate-8.1.0.crate" >&2
            tar -xzf "$cache/$crate.crate" -C "$cache/$crate" --strip-components=1
        fi
    done
    git apply --check --directory="$cache" "$patch_file"
    git apply --directory="$cache" "$patch_file"
    touch "$cache/.ready"
fi
printf '%s/%s\n' "$PWD" "$cache"
