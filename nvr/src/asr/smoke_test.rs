//! Ignored end-to-end smoke for the ASR audio path (no ZLM, no Socket.IO).
//!
//! Drives a `Pipe` from a speech audio file, taps its decoded audio via
//! `Pipe::subscribe_audio`, resamples to 16 kHz mono, and transcribes with the
//! shared models — exercising the T3/T4 (Bus/Pipe audio tap) + T5 (resample) +
//! T1 (engine) integration on real pipeline audio.
//!
//! Requires the models under `third_party/asr-models` (or `$ASR_MODELS_DIR`) and
//! a **speech** media file in `$ASR_SMOKE_MEDIA`. Run:
//! ```text
//! SHERPA_ONNX_LIB_DIR=.../lib LD_LIBRARY_PATH=$PWD/ffmpeg/lib \
//! ASR_SMOKE_MEDIA=/path/speech.m4a \
//! cargo test -p nvr -- --ignored asr_smoke --nocapture
//! ```

use std::sync::Arc;
use std::time::Duration;

use ffmpeg_bus::frame::{RawFrame, RawFrameCmd};
use media_pipe_core::{Pipe, PipeConfig};
use nvr_asr::{AsrConfig, AsrEngine, AsrModels, Transcript};

use super::resample::Pcm16kMono;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs models + a speech file in $ASR_SMOKE_MEDIA"]
async fn asr_smoke_transcribes_pipe_audio() {
    let media =
        std::env::var("ASR_SMOKE_MEDIA").expect("set ASR_SMOKE_MEDIA to a speech media file");
    let base = std::path::PathBuf::from(
        std::env::var("ASR_MODELS_DIR").unwrap_or_else(|_| "third_party/asr-models".into()),
    );
    let sv = base.join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17");
    let punct = base.join("sherpa-onnx-punct-ct-transformer-zh-en-vocab272727-2024-04-12");
    let mut cfg = AsrConfig::new(
        sv.join("model.int8.onnx"),
        sv.join("tokens.txt"),
        base.join("silero_vad.onnx"),
    );
    if punct.join("model.onnx").exists() {
        cfg.punct_model = Some(punct.join("model.onnx"));
    }
    let models = AsrModels::load(cfg).expect("load models");

    // Start a no-output pipe reading the speech file.
    let pipe = Arc::new(Pipe::new(PipeConfig::builder().input_file(media).build()));
    {
        let p = pipe.clone();
        tokio::spawn(async move { p.start(None).await });
    }

    // subscribe_audio races pipe startup (input not yet demuxed); retry briefly.
    let mut rx = {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            match pipe.subscribe_audio().await {
                Ok(rx) => break rx,
                Err(_) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => panic!("subscribe_audio never became ready: {e:#}"),
            }
        }
    };

    let mut engine = AsrEngine::new(models).expect("engine");
    let mut resampler = Pcm16kMono::new();

    // Phase 1 — drain the tap fast, resampling to 16 kHz mono. A file input
    // floods faster than realtime, so we must NOT run ASR inline here: that
    // out-races the broadcast and shows up as `Lagged` (dropped frames). Just
    // collect, and continue on `Lagged` exactly like the real tap does.
    use tokio::sync::broadcast::error::RecvError;
    let mut samples: Vec<f32> = Vec::new();
    let mut lagged = 0u64;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Ok(RawFrameCmd::Data(RawFrame::Audio(f)))) => {
                if let Ok(s) = resampler.push(f.as_audio()) {
                    samples.extend_from_slice(&s);
                }
            }
            Ok(Ok(RawFrameCmd::Data(RawFrame::Video(_)))) => {}
            Ok(Ok(RawFrameCmd::EOF)) => break,
            Ok(Err(RecvError::Lagged(n))) => lagged += n,
            Ok(Err(RecvError::Closed)) => break,
            Err(_) => break, // 3s gap => stream ended
        }
    }
    pipe.cancel();

    // Phase 2 — feed the collected audio to the engine sequentially in ~200 ms
    // chunks (as the demo does), so VAD sees continuous, correctly-ordered audio.
    let mut finals: Vec<String> = Vec::new();
    for chunk in samples.chunks(3200) {
        for t in engine.accept(chunk) {
            if let Transcript::Final { text, .. } = t {
                finals.push(text);
            }
        }
    }
    for t in engine.flush() {
        if let Transcript::Final { text, .. } = t {
            finals.push(text);
        }
    }

    eprintln!(
        "[asr_smoke] tapped {} samples @16k mono (lagged={lagged}); finals = {finals:?}",
        samples.len()
    );
    assert!(
        samples.len() > 8_000,
        "expected decoded audio to flow through the tap, got {} samples",
        samples.len()
    );
    assert!(!finals.is_empty(), "expected at least one Final transcript");
}
