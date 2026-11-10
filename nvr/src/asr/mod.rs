//! Real-time ASR for live pipes: taps decoded audio, transcribes via `nvr-asr`,
//! and emits transcripts over Socket.IO. Opt-in per pipe.

pub mod api;
pub mod hub;
pub mod resample;
pub mod tap;

use std::path::PathBuf;

use nvr_asr::AsrConfig;
use socketioxide::SocketIo;
use socketioxide::extract::{Data, SocketRef};
use socketioxide::layer::SocketIoLayer;

/// Build the Socket.IO layer + handle, registering the `/asr` namespace.
///
/// Clients emit `subscribe`/`unsubscribe` with a pipe id (string) to join/leave
/// that pipe's transcript room.
pub fn build_socketio() -> (SocketIoLayer, SocketIo) {
    let (layer, io) = SocketIo::new_layer();
    io.ns("/asr", async |s: SocketRef| {
        s.on("subscribe", async |s: SocketRef, Data::<String>(pipe)| {
            s.join(pipe);
        });
        s.on("unsubscribe", async |s: SocketRef, Data::<String>(pipe)| {
            s.leave(pipe);
        });
    });
    (layer, io)
}

/// Resolve ASR model paths from env, defaulting to the `third_party/asr-models`
/// layout from `make download-asr-models`.
pub fn model_config() -> AsrConfig {
    let base =
        std::env::var("ASR_MODELS_DIR").unwrap_or_else(|_| "third_party/asr-models".to_string());
    let base = PathBuf::from(base);
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
    if let Ok(secs) = std::env::var("ASR_MAX_SPEECH_SECS") {
        match secs.trim().parse::<f32>() {
            Ok(secs) if secs > 0.0 => cfg.max_speech_duration = secs,
            _ => log::warn!("ignoring ASR_MAX_SPEECH_SECS={secs:?}: expected positive seconds"),
        }
    }
    if let Ok(v) = std::env::var("ASR_EARLY_COMMIT") {
        match v.trim() {
            "0" | "false" | "off" => cfg.early_commit = false,
            "1" | "true" | "on" => cfg.early_commit = true,
            _ => log::warn!("ignoring ASR_EARLY_COMMIT={v:?}: expected 0|1"),
        }
    }
    if let Ok(mode) = std::env::var("ASR_PARTIAL_PUNCT") {
        match mode.parse() {
            Ok(mode) => cfg.partial_punct = mode,
            Err(e) => log::warn!("ignoring ASR_PARTIAL_PUNCT: {e:#}"),
        }
    }
    cfg
}

#[cfg(test)]
#[path = "resample_test.rs"]
mod resample_test;

#[cfg(test)]
#[path = "smoke_test.rs"]
mod smoke_test;
