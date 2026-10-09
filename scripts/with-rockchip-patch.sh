#!/usr/bin/env bash
# Apply Cargo path overrides only for this Rockchip command; restore workspace files.
# Usage: bash scripts/with-rockchip-patch.sh cargo|cross build|test|check ...
set -euo pipefail
cd "$(dirname "$0")/.."
command_name="$1"
shift
mkdir -p .cache
build_lock=.cache/rockchip-build.lock
if ! mkdir "$build_lock" 2>/dev/null; then
    echo 'Another Rockchip build is running; wait for it to finish.' >&2
    exit 1
fi
lock_backup=$(mktemp .cache/rockchip-Cargo.lock.XXXXXX)
config_backup=$(mktemp .cache/rockchip-config.XXXXXX)
cp Cargo.lock "$lock_backup"
cp .cargo/config.toml "$config_backup"
restore_workspace() {
    cp "$lock_backup" Cargo.lock
    cp "$config_backup" .cargo/config.toml
    rm -f "$lock_backup" "$config_backup"
    rmdir "$build_lock"
}
trap restore_workspace EXIT
cache=$(bash scripts/prepare-rockchip-patch.sh)
cache_relative="${cache#"$PWD/"}"
# Cross runs its own metadata command without forwarding CLI --config overrides.
# A temporary workspace config covers both that command and container compilation.
cat >> .cargo/config.toml <<EOF

[patch.crates-io]
ffmpeg-next = { path = "$cache_relative/ffmpeg-next" }
ffmpeg-sys-next = { path = "$cache_relative/ffmpeg-sys-next" }
EOF
cargo metadata --format-version 1 >/dev/null
"$command_name" "$@"
