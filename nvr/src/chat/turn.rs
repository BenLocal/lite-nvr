//! One question → answer turn: stream the LLM reply as text and speak it
//! sentence by sentence while it is still being generated.

use bytes::Bytes;
use futures::StreamExt;
use nvr_rkllm::{ChatRequest, Delta, GenerateError, Message};
use nvr_tts::SentenceSplitter;
use serde::Serialize;
use serde_json::json;
use socketioxide::extract::SocketRef;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

use super::pcm::f32_to_s16le;
use super::session::Event;
use crate::llm::Llm;

#[derive(Serialize)]
struct ReplyAudio {
    turn: u64,
    /// Order of this sentence within the turn, from 0.
    seq: u32,
    sample_rate: u32,
    /// Mono s16le.
    pcm: Bytes,
}

/// Runs until the reply has been generated and spoken, or `cancel` fires.
/// Cancelling drops the generation, which stops the LLM.
pub(super) async fn run(
    socket: SocketRef,
    turn: u64,
    question: String,
    messages: Vec<Message>,
    cancel: CancellationToken,
    events: UnboundedSender<Event>,
) {
    let result = tokio::select! {
        _ = cancel.cancelled() => return,
        result = reply(&socket, turn, messages) => result,
    };
    match result {
        Ok(reply) => {
            let _ = events.send(Event::TurnDone {
                turn,
                question,
                reply,
            });
        }
        Err(message) => {
            let _ = socket.emit("error", &json!({ "turn": turn, "message": message }));
        }
    }
    let _ = socket.emit("reply_done", &json!({ "turn": turn }));
}

async fn reply(socket: &SocketRef, turn: u64, messages: Vec<Message>) -> Result<String, String> {
    let engine = match crate::llm::get() {
        Llm::Enabled { engine, .. } => engine,
        Llm::Disabled => return Err("LLM is disabled (NVR_LLM_MODEL is not set)".into()),
        Llm::Invalid(reason) => return Err(format!("LLM is misconfigured: {reason}")),
    };
    let request = ChatRequest {
        messages,
        // Thinking would delay the first spoken sentence.
        enable_thinking: false,
        ..Default::default()
    };
    let mut generation = engine.generate(request).map_err(|err| match err {
        GenerateError::Busy => "model is busy, retry later".to_string(),
        err => err.to_string(),
    })?;

    let (sentences, mut to_speak) = unbounded_channel::<String>();
    let generate = async move {
        let mut reply = String::new();
        let mut splitter = SentenceSplitter::new();
        while let Some(delta) = generation.next().await {
            match delta {
                Delta::Reasoning(_) => {}
                Delta::Content(text) => {
                    let _ = socket.emit("reply_delta", &json!({ "turn": turn, "text": text }));
                    reply.push_str(&text);
                    for sentence in splitter.push(&text) {
                        let _ = sentences.send(sentence);
                    }
                }
                Delta::Done { .. } => break,
                Delta::Error(err) => return Err(err),
            }
        }
        if let Some(rest) = splitter.finish() {
            let _ = sentences.send(rest);
        }
        Ok(reply)
    };
    let speak = async move {
        let tts = super::tts::get().await;
        if let Err(err) = &tts {
            let message = format!("TTS unavailable: {err}");
            let _ = socket.emit("error", &json!({ "turn": turn, "message": message }));
        }
        let mut seq = 0;
        // Drain even without TTS so the reply text is unaffected.
        while let Some(sentence) = to_speak.recv().await {
            let Ok(tts) = &tts else { continue };
            match super::tts::synthesize(tts, sentence).await {
                Ok(audio) if !audio.samples.is_empty() => {
                    let data = ReplyAudio {
                        turn,
                        seq,
                        sample_rate: audio.sample_rate,
                        pcm: Bytes::from(f32_to_s16le(&audio.samples)),
                    };
                    let _ = socket.emit("reply_audio", &data);
                    seq += 1;
                }
                Ok(_) => {}
                Err(err) => log::warn!("chat: TTS failed: {err:#}"),
            }
        }
    };
    let (reply, ()) = tokio::join!(generate, speak);
    reply
}
