#!/usr/bin/env bash
#
# Download the ASR *models* that crates/nvr-asr needs at runtime (distinct from
# the native libs fetched by download_sherpa_onnx_libs.sh):
#   - silero_vad.onnx                     Silero VAD (speech segmentation)
#   - <sense-voice model>/model.int8.onnx SenseVoice offline recognizer
#   - <sense-voice model>/tokens.txt      recognizer tokens
#   - <sense-voice model>/test_wavs/*.wav bundled 16 kHz sample clips
#
# Honors http_proxy/https_proxy from the environment (e.g. `make
# download-asr-models`, which exports them from .env). Idempotent + resumable.
#
# Usage:
#   scripts/download_asr_models.sh [--dest <dir>] [--model <release-name>]
#
# Env overrides: DEST, SENSE_VOICE_MODEL, ASR_MODELS_BASE_URL.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

DEST="${DEST:-$REPO_ROOT/third_party/asr-models}"
SENSE_VOICE_MODEL="${SENSE_VOICE_MODEL:-sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17}"
BASE="${ASR_MODELS_BASE_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models}"
# The punctuation model lives under a separate GitHub release tag.
PUNCT_BASE="${PUNCT_MODELS_BASE_URL:-https://github.com/k2-fsa/sherpa-onnx/releases/download/punctuation-models}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dest)  DEST="$2"; shift ;;
    --model) SENSE_VOICE_MODEL="$2"; shift ;;
    -h|--help)
      sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//; s/^#$//'
      exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
  shift
done

mkdir -p "$DEST"

# Resumable download helper (curl preferred, wget fallback).
dl() {
  local url="$1" out="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fL --retry 5 --retry-delay 3 --retry-connrefused -C - -o "$out" "$url"
  elif command -v wget >/dev/null 2>&1; then
    wget --continue --tries=5 --waitretry=3 -O "$out" "$url"
  else
    echo "Need curl or wget." >&2; exit 1
  fi
}

# 1) Silero VAD (~0.6 MB)
VAD="$DEST/silero_vad.onnx"
if [[ -f "$VAD" ]]; then
  echo "VAD already present: $VAD"
else
  echo "Downloading silero_vad.onnx ..."
  dl "$BASE/silero_vad.onnx" "$VAD"
fi

# 2) SenseVoice model. The release tarball is ~1 GB: it bundles the fp32 model
#    (unused here), the int8 model we use, tokens.txt and test_wavs/.
MODEL_DIR="$DEST/$SENSE_VOICE_MODEL"
if [[ -f "$MODEL_DIR/model.int8.onnx" && -f "$MODEL_DIR/tokens.txt" ]]; then
  echo "SenseVoice model already present: $MODEL_DIR"
else
  TARBALL="$DEST/$SENSE_VOICE_MODEL.tar.bz2"
  echo "Downloading $SENSE_VOICE_MODEL.tar.bz2 (~1 GB, resumable) ..."
  dl "$BASE/$SENSE_VOICE_MODEL.tar.bz2" "$TARBALL"
  echo "Verifying + extracting ..."
  bzip2 -t "$TARBALL"
  tar -xjf "$TARBALL" -C "$DEST"
  rm -f "$TARBALL"   # extracted; drop the 1 GB archive
fi

# 3) Punctuation model (CT-Transformer, zh/en). Used to punctuate Finals only.
PUNCT_MODEL="${PUNCT_MODEL:-sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12}"
PUNCT_DIR="$DEST/$PUNCT_MODEL"
if [[ -f "$PUNCT_DIR/model.onnx" ]]; then
  echo "Punctuation model already present: $PUNCT_DIR"
else
  TARBALL="$DEST/$PUNCT_MODEL.tar.bz2"
  echo "Downloading $PUNCT_MODEL.tar.bz2 ..."
  dl "$PUNCT_BASE/$PUNCT_MODEL.tar.bz2" "$TARBALL"
  bzip2 -t "$TARBALL"
  tar -xjf "$TARBALL" -C "$DEST"
  rm -f "$TARBALL"
fi

echo
echo "==================================================================="
echo "Models ready under $DEST"
echo "  VAD    : $VAD"
echo "  model  : $MODEL_DIR/model.int8.onnx"
echo "  tokens : $MODEL_DIR/tokens.txt"
echo "  wavs   : $MODEL_DIR/test_wavs/"
echo "  punct  : $PUNCT_DIR/model.onnx"
echo
echo "Run the demo (with punctuation on finals only):"
echo "  cargo run -p nvr-asr --bin nvr-asr-demo -- \\"
echo "    --model  $MODEL_DIR/model.int8.onnx \\"
echo "    --tokens $MODEL_DIR/tokens.txt \\"
echo "    --vad    $VAD \\"
echo "    --punct  $PUNCT_DIR/model.onnx \\"
echo "    --wav    $MODEL_DIR/test_wavs/zh.wav"
echo "==================================================================="
