#!/usr/bin/env bash
set -euo pipefail

DEFAULT_FFMPEG_URL="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n7.1-latest-linux64-gpl-shared-7.1.tar.xz"
FFMPEG_URL="${1:-${FFMPEG_URL:-${DEFAULT_FFMPEG_URL}}}"

download_and_extract_ffmpeg() {
  local root_dir ffmpeg_dir archive_pathhttp://172.31.169.114:1234/shiben/ffmpeg/ffmpeg-n7.1-latest-linux64-gpl-shared-7.1.tar.xz

  root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  ffmpeg_dir="${root_dir}/ffmpeg"
  archive_path="${ffmpeg_dir}/ffmpeg-shared.tar.xz"

  mkdir -p "${ffmpeg_dir}"

  if command -v curl >/dev/null 2>&1; then
    curl -L "${FFMPEG_URL}" -o "${archive_path}"
  elif command -v wget >/dev/null 2>&1; then
    wget -O "${archive_path}" "${FFMPEG_URL}"
  else
    echo "Error: curl or wget is required to download FFmpeg." >&2
    exit 1
  fi

  tar -xJf "${archive_path}" -C "${ffmpeg_dir}" --strip-components=1
  rm -f "${archive_path}"
}

download_and_extract_ffmpeg
