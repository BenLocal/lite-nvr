#!/usr/bin/env bash
#
# Download the prebuilt sherpa-onnx native libraries that the `sherpa-onnx`
# Rust crate (used by crates/nvr-asr) links against, and print the
# SHERPA_ONNX_LIB_DIR export line.
#
# Why: sherpa-onnx-sys's build script otherwise downloads these archives from
# GitHub on every fresh target dir. Pre-fetching once and pointing
# SHERPA_ONNX_LIB_DIR at the extracted `lib/` makes the build offline,
# resumable, and reproducible.
#
# Usage:
#   scripts/download_sherpa_onnx_libs.sh [--shared|--static] [options]
#
# Options:
#   --static            Static libs (.a) — default; self-contained binary,
#                       no runtime LD_LIBRARY_PATH. Matches the crate default.
#   --shared            Shared libs (.so). Requires building nvr-asr with the
#                       crate's `shared` feature (see note printed at the end).
#   --version <X.Y.Z>   sherpa-onnx version. Default: auto-detected from
#                       Cargo.lock, else 1.13.3.
#   --arch <x86_64|aarch64>   Target arch. Default: host arch.
#   --os <linux|macos|windows>   Target OS. Default: host OS.
#   --dest <dir>        Where to extract. Default: <repo>/third_party/sherpa-onnx
#   -h, --help          Show this help.
#
# Env overrides: VERSION, LINK, ARCH, OS, DEST behave like the flags above.
set -euo pipefail

# --- locate repo root (this script lives in <repo>/scripts) ---------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- defaults -------------------------------------------------------------
LINK="${LINK:-static}"
VERSION="${VERSION:-}"
ARCH="${ARCH:-}"
OS="${OS:-}"
DEST="${DEST:-$REPO_ROOT/third_party/sherpa-onnx}"

usage() { sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//; s/^#$//'; exit "${1:-0}"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --static) LINK="static" ;;
    --shared) LINK="shared" ;;
    --version) VERSION="$2"; shift ;;
    --arch)    ARCH="$2"; shift ;;
    --os)      OS="$2"; shift ;;
    --dest)    DEST="$2"; shift ;;
    -h|--help) usage 0 ;;
    *) echo "Unknown argument: $1" >&2; usage 1 ;;
  esac
  shift
done

# --- auto-detect version from Cargo.lock (fall back to 1.13.3) ------------
if [[ -z "$VERSION" ]]; then
  if [[ -f "$REPO_ROOT/Cargo.lock" ]]; then
    VERSION="$(awk '
      /^name = "sherpa-onnx-sys"/ { hit=1; next }
      hit && /^version = / { gsub(/[",]/,"",$3); print $3; exit }
    ' "$REPO_ROOT/Cargo.lock" || true)"
  fi
  VERSION="${VERSION:-1.13.3}"
fi

# --- auto-detect host os/arch --------------------------------------------
if [[ -z "$OS" ]]; then
  case "$(uname -s)" in
    Linux)  OS="linux" ;;
    Darwin) OS="macos" ;;
    MINGW*|MSYS*|CYGWIN*) OS="windows" ;;
    *) echo "Cannot detect OS from '$(uname -s)'; pass --os" >&2; exit 1 ;;
  esac
fi
if [[ -z "$ARCH" ]]; then
  case "$(uname -m)" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) echo "Cannot detect arch from '$(uname -m)'; pass --arch" >&2; exit 1 ;;
  esac
fi

# --- map (LINK, OS, ARCH) -> archive name (must match sherpa-onnx-sys/build.rs) ---
archive_name() {
  case "$LINK:$OS:$ARCH" in
    static:linux:x86_64)   echo "sherpa-onnx-v${VERSION}-linux-x64-static-lib.tar.bz2" ;;
    static:linux:aarch64)  echo "sherpa-onnx-v${VERSION}-linux-aarch64-static-lib.tar.bz2" ;;
    static:macos:x86_64)   echo "sherpa-onnx-v${VERSION}-osx-x64-static-lib.tar.bz2" ;;
    static:macos:aarch64)  echo "sherpa-onnx-v${VERSION}-osx-arm64-static-lib.tar.bz2" ;;
    static:windows:x86_64) echo "sherpa-onnx-v${VERSION}-win-x64-static-MT-Release-lib.tar.bz2" ;;
    shared:linux:x86_64)   echo "sherpa-onnx-v${VERSION}-linux-x64-shared-lib.tar.bz2" ;;
    shared:linux:aarch64)  echo "sherpa-onnx-v${VERSION}-linux-aarch64-shared-cpu-lib.tar.bz2" ;;
    shared:macos:x86_64)   echo "sherpa-onnx-v${VERSION}-osx-x64-shared-lib.tar.bz2" ;;
    shared:macos:aarch64)  echo "sherpa-onnx-v${VERSION}-osx-arm64-shared-lib.tar.bz2" ;;
    shared:windows:x86_64) echo "sherpa-onnx-v${VERSION}-win-x64-shared-MT-Release-lib.tar.bz2" ;;
    *) echo "" ;;
  esac
}

ARCHIVE="$(archive_name)"
if [[ -z "$ARCHIVE" ]]; then
  echo "Unsupported combination: LINK=$LINK OS=$OS ARCH=$ARCH" >&2
  exit 1
fi
STEM="${ARCHIVE%.tar.bz2}"
BASE_URL="https://github.com/k2-fsa/sherpa-onnx/releases/download"
URL="$BASE_URL/v${VERSION}/${ARCHIVE}"
LIB_DIR="$DEST/$STEM/lib"

echo "sherpa-onnx prebuilt libs"
echo "  version : $VERSION"
echo "  target  : $OS/$ARCH ($LINK)"
echo "  archive : $ARCHIVE"
echo "  url     : $URL"
echo "  dest    : $DEST"
echo

# --- already present? -----------------------------------------------------
if [[ -d "$LIB_DIR" ]] && ls "$LIB_DIR"/lib* >/dev/null 2>&1; then
  echo "Already extracted at $LIB_DIR — skipping download."
else
  mkdir -p "$DEST"
  ARCHIVE_PATH="$DEST/$ARCHIVE"

  # Resumable download: prefer curl, fall back to wget. Retries on flaky net.
  if command -v curl >/dev/null 2>&1; then
    echo "Downloading (curl, resumable)…"
    curl -fL --retry 5 --retry-delay 3 --retry-connrefused -C - \
      -o "$ARCHIVE_PATH" "$URL"
  elif command -v wget >/dev/null 2>&1; then
    echo "Downloading (wget, resumable)…"
    wget --continue --tries=5 --waitretry=3 -O "$ARCHIVE_PATH" "$URL"
  else
    echo "Need curl or wget to download." >&2
    exit 1
  fi

  echo "Extracting…"
  tar -xjf "$ARCHIVE_PATH" -C "$DEST"

  if [[ ! -d "$LIB_DIR" ]]; then
    echo "Extracted archive has no lib/ dir at $LIB_DIR" >&2
    exit 1
  fi
  echo "Extracted to $DEST/$STEM"
fi

echo
echo "Libraries:"
ls -1 "$LIB_DIR" | sed 's/^/  /'
echo
echo "==================================================================="
echo "Point the build at these libs (add to your shell profile to persist):"
echo
echo "  export SHERPA_ONNX_LIB_DIR=\"$LIB_DIR\""
if [[ "$LINK" == "shared" ]]; then
  echo "  export LD_LIBRARY_PATH=\"$LIB_DIR:\$LD_LIBRARY_PATH\""
  echo
  echo "NOTE: shared linking needs the crate's \`shared\` feature. In"
  echo "      crates/nvr-asr/Cargo.toml set:"
  echo "        sherpa-onnx = { version = \"$VERSION\", default-features = false, features = [\"shared\"] }"
fi
echo
echo "Then build:  SHERPA_ONNX_LIB_DIR=\"$LIB_DIR\" cargo build -p nvr-asr"
echo "==================================================================="
