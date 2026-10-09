#!/usr/bin/env bash
# Runs as root inside the official cross build image.
set -euo pipefail
if [[ -n "${APT_MIRROR:-}" ]]; then
    case "$APT_MIRROR" in
        http://*|https://*) ;;
        *) echo 'APT_MIRROR must be an http(s) Ubuntu archive URL' >&2; exit 1 ;;
    esac
    mirror="${APT_MIRROR%/}"
    escaped_mirror=$(printf '%s' "$mirror" | sed 's/[&|\\]/\\&/g')
    for source in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
        [[ -f "$source" ]] || continue
        sed -i -E "s|https?://(archive.ubuntu.com/ubuntu/?\|security.ubuntu.com/ubuntu/?)|$escaped_mirror|g" "$source"
    done
    ports_mirror="${mirror%/ubuntu}/ubuntu-ports"
    escaped_ports=$(printf '%s' "$ports_mirror" | sed 's/[&|\\]/\\&/g')
    for source in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
        [[ -f "$source" ]] || continue
        sed -i -E "s|https?://ports.ubuntu.com/ubuntu-ports/?|$escaped_ports|g" "$source"
    done
fi
# Official images already provide these tools; only install when absent.
if command -v pkg-config >/dev/null && compgen -G '/usr/lib/llvm-*/lib/libclang.so*' >/dev/null; then
    exit 0
fi
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends libclang-dev pkg-config
