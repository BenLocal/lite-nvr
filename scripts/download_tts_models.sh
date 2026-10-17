#!/usr/bin/env bash
#
# Download the TTS models that crates/nvr-tts needs at runtime:
#   vits-melo-tts-zh_en/  sherpa-onnx CPU model: model.onnx (+ model.int8.onnx
#                         with --int8; measured slower), lexicon.txt,
#                         tokens.txt, *.fst, dict/
#   MeloTTS-RKNN2/        with --melo-rknn: the same voice for Rockchip NPUs
#                         (encoder.onnx, decoder.rknn, g.bin, lexicon.txt,
#                         tokens.txt). These files are AGPL-3.0
#                         (happyme531/MeloTTS-RKNN2) and are never vendored.
#
# Files come from Hugging Face. Set HF_ENDPOINT=https://hf-mirror.com to use
# the mirror (no proxy needed in China); otherwise http_proxy/https_proxy from
# the environment are honored. Idempotent + resumable.
#
# Usage:
#   scripts/download_tts_models.sh [--dest <dir>] [--int8] [--melo-rknn]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

DEST="${DEST:-$REPO_ROOT/third_party/tts-models}"
ENDPOINT="${HF_ENDPOINT:-https://huggingface.co}"
INT8=0
MELO_RKNN=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dest) DEST="$2"; shift ;;
    --int8) INT8=1 ;;
    --melo-rknn) MELO_RKNN=1 ;;
    -h|--help)
      sed -n '2,/^set -euo/p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//; s/^#$//'
      exit 0 ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
  shift
done

# fetch <hf repo> <local dir> <file>...
fetch() {
  local repo="$1" out_dir="$2"
  shift 2
  for f in "$@"; do
    local out="$out_dir/$f"
    if [[ -s "$out" ]]; then
      echo "present: $f"
      continue
    fi
    mkdir -p "$(dirname "$out")"
    echo "downloading $f ..."
    curl -fL --progress-bar --retry 5 --retry-delay 3 --retry-connrefused -C - -o "$out.part" \
      "$ENDPOINT/$repo/resolve/main/$f"
    mv "$out.part" "$out"
  done
  echo "ready: $out_dir"
}

VITS=(
  model.onnx lexicon.txt tokens.txt
  date.fst number.fst phone.fst new_heteronym.fst
  dict/hmm_model.utf8 dict/idf.utf8 dict/jieba.dict.utf8 dict/stop_words.utf8 dict/user.dict.utf8
  dict/pos_dict/char_state_tab.utf8 dict/pos_dict/prob_emit.utf8
  dict/pos_dict/prob_start.utf8 dict/pos_dict/prob_trans.utf8
)
if [[ "$INT8" == 1 ]]; then VITS+=(model.int8.onnx); fi
fetch csukuangfj/vits-melo-tts-zh_en "$DEST/vits-melo-tts-zh_en" "${VITS[@]}"

if [[ "$MELO_RKNN" == 1 ]]; then
  fetch happyme531/MeloTTS-RKNN2 "$DEST/MeloTTS-RKNN2" \
    encoder.onnx decoder.rknn g.bin lexicon.txt tokens.txt
fi
