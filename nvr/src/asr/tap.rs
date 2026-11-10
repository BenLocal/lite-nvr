//! Per-pipe ASR tap: decoded audio -> resample -> engine -> Socket.IO room.

use std::sync::Arc;

use ffmpeg_bus::frame::{RawFrame, RawFrameCmd, RawFrameReceiver};
use nvr_asr::{AsrEngine, AsrModels, Transcript, split_sentences};
use serde_json::json;
use socketioxide::SocketIo;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use super::resample::Pcm16kMono;

/// Drive one pipe's transcription until `cancel` fires or the audio broadcast
/// ends. `audio` is a subscription obtained from `Pipe::subscribe_audio`.
pub async fn run(
    pipe: String,
    models: Arc<AsrModels>,
    mut audio: RawFrameReceiver,
    io: SocketIo,
    cancel: CancellationToken,
) {
    let mut engine = match AsrEngine::new(models) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("asr[{pipe}]: engine init failed: {e:#}");
            return;
        }
    };
    let mut resampler = Pcm16kMono::new();

    loop {
        let cmd = tokio::select! {
            _ = cancel.cancelled() => break,
            r = audio.recv() => r,
        };
        match cmd {
            Ok(RawFrameCmd::Data(RawFrame::Audio(frame))) => {
                let samples = match resampler.push(frame.as_audio()) {
                    Ok(s) => s,
                    Err(e) => {
                        log::debug!("asr[{pipe}]: resample error: {e:#}");
                        continue;
                    }
                };
                for t in engine.accept(&samples) {
                    emit(&io, &pipe, &t).await;
                }
            }
            Ok(RawFrameCmd::Data(RawFrame::Video(_))) => {} // ignore video
            Ok(RawFrameCmd::EOF) => break,
            Err(RecvError::Lagged(n)) => {
                log::debug!("asr[{pipe}]: dropped {n} audio frames (lag)");
            }
            Err(RecvError::Closed) => break,
        }
    }

    // Flush any trailing utterance on shutdown.
    for t in engine.flush() {
        emit(&io, &pipe, &t).await;
    }
    log::info!("asr[{pipe}]: tap stopped");
}

/// Room event for one transcript. `id` is the utterance id: a `final` replaces
/// every `partial` with the same id, and `sentences` splits it into short lines.
fn payload(pipe: &str, t: &Transcript) -> (&'static str, serde_json::Value) {
    match t {
        Transcript::Partial { segment, text } => (
            "partial",
            json!({ "pipe": pipe, "id": segment, "final": false, "text": text }),
        ),
        Transcript::Final {
            segment,
            text,
            start,
            duration,
        } => {
            let sentences: Vec<_> = split_sentences(text, *start, *duration)
                .into_iter()
                .map(|s| json!({ "text": s.text, "start": s.start, "duration": s.duration }))
                .collect();
            (
                "final",
                json!({
                    "pipe": pipe,
                    "id": segment,
                    "final": true,
                    "text": text,
                    "start": start,
                    "duration": duration,
                    "sentences": sentences,
                }),
            )
        }
    }
}

async fn emit(io: &SocketIo, pipe: &str, t: &Transcript) {
    let Some(ns) = io.of("/asr") else { return };
    let (event, data) = payload(pipe, t);
    let _ = ns.to(pipe.to_string()).emit(event, &data).await;
}

#[cfg(test)]
#[path = "tap_test.rs"]
mod tap_test;
