# nvr-asr

Real-time speech-to-text that uses an **offline** ASR model to *simulate
streaming*, with correction.

A streaming (online) recognizer emits words as they arrive but is typically
less accurate. Instead, this crate pairs a **Silero VAD** with an **offline
SenseVoice** recognizer:

1. The VAD consumes the audio and detects speech boundaries.
2. While a speech segment is still open, the offline recognizer re-decodes the
   accumulated audio every ~300 ms, emitting an interim `Partial` that
   **supersedes and may correct** the previous one.
3. When the VAD closes the segment, the finalized (silence-trimmed) samples are
   decoded into a stable `Final`.

The engine is source-agnostic: feed it 16 kHz mono `f32` PCM.

```
audio 16k/mono/f32 ──▶ AsrEngine.accept() ──▶ [Transcript::Partial ...]   (self-correcting, live)
                                          └─▶ Transcript::Final { text, start, duration }
```

## Layout

| File | Role |
|------|------|
| `src/lib.rs` | `AsrConfig`, `Transcript`, public re-exports |
| `src/engine.rs` | `AsrEngine` — VAD + offline recognizer, the streaming-correction loop |
| `src/wav.rs` | WAV → mono `f32` loader (demo input) |
| `src/main.rs` | `nvr-asr-demo` binary — WAV driver that simulates a live stream |

## Native library

`sherpa-onnx` binds to a native library (the sherpa-onnx C API + ONNX Runtime).
It links **statically** by default, and its build script downloads the matching
prebuilt static archive from
[GitHub Releases](https://github.com/k2-fsa/sherpa-onnx/releases) the first time
you build. With static linking the resulting binary is self-contained — no
runtime `LD_LIBRARY_PATH`.

To make the build **offline / reproducible** (and avoid re-downloading per
target dir), pre-fetch the libs and point `SHERPA_ONNX_LIB_DIR` at them:

```bash
# downloads + extracts to third_party/sherpa-onnx/, prints the export line
scripts/download_sherpa_onnx_libs.sh
export SHERPA_ONNX_LIB_DIR="$PWD/third_party/sherpa-onnx/sherpa-onnx-v1.13.3-linux-x64-static-lib/lib"
```

When `SHERPA_ONNX_LIB_DIR` is set to a directory containing the libs, the build
script uses it directly and skips the download. The script auto-detects your
OS/arch and the version from `Cargo.lock`; run it with `--help` for options
(e.g. `--shared`, `--arch aarch64`).

> Alternatives the build script also honors: `SHERPA_ONNX_ARCHIVE_DIR` (a
> directory holding the pre-downloaded `.tar.bz2`), or just letting it download
> on first build if you have network access.

## Models

The recognizer needs a SenseVoice model (ships with `tokens.txt`) and the
Silero VAD model. Fetch both into `third_party/asr-models/` with:

```bash
make download-asr-models        # honors HTTP(S)_PROXY from .env
```

The SenseVoice release tarball also bundles `test_wavs/*.wav` (16 kHz sample
clips) you can point the demo at.

<details><summary>Manual download</summary>

```bash
# SenseVoice (multilingual: zh / en / ja / ko / yue)
curl -LO https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2
tar xjf sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17.tar.bz2
# Silero VAD
curl -LO https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx
```

</details>

## Run the demo

The demo expects a **16 kHz mono** WAV (no resampling is done). Convert if needed:

```bash
ffmpeg -i input.wav -ar 16000 -ac 1 speech-16k.wav
```

```bash
# SHERPA_ONNX_LIB_DIR exported per the "Native library" section above
cargo run -p nvr-asr --bin nvr-asr-demo -- \
  --model  sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/model.int8.onnx \
  --tokens sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/tokens.txt \
  --vad    silero_vad.onnx \
  --wav    speech-16k.wav \
  --language auto \
  --realtime          # pace chunks to wall-clock to feel like a live mic
```

Interim lines overwrite in place (`… 今天天气`) and commit on segment end
(`[  1.20s +2.34s] 今天天气不错`).

## Library usage

```rust
use nvr_asr::{AsrConfig, AsrEngine, Transcript};

let mut cfg = AsrConfig::new("model.int8.onnx", "tokens.txt", "silero_vad.onnx");
cfg.language = "zh".into();
let mut engine = AsrEngine::new(cfg)?;

// pcm: &[f32], 16 kHz mono, arriving in arbitrarily-sized chunks
for chunk in stream {
    for ev in engine.accept(chunk) {
        match ev {
            Transcript::Partial { text }              => update_live_caption(&text),
            Transcript::Final { text, start, .. }     => commit_caption(start, &text),
        }
    }
}
for ev in engine.flush() { /* trailing finals */ }
```

## Tests

`cargo test -p nvr-asr` covers the pure-Rust helpers (config timing, WAV
downmix). End-to-end transcription needs the native library **and** the models,
so it is exercised via the demo, not unit tests.
