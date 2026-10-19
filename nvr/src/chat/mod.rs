//! Voice chat robot (Rockchip builds): the Socket.IO `/chat` namespace runs
//! push-to-talk speech through ASR, the on-board LLM and TTS, and streams the
//! reply back as text and per-sentence audio. See `docs/voice-chat-spec.md`.
//!
//! Client → server: `start` / `stop` (push to talk), `audio` (binary, 16 kHz
//! mono s16le), `text` (typed question), `cancel`, `reset`.
//! Server → client: `asr_partial` / `asr_final`, `reply_delta`, `reply_audio`
//! (binary s16le with `turn`, `seq`, `sample_rate`), `reply_done`, `error`.
//! The `text` ack and `asr_final` report the `turn` that answers the question
//! (null when none starts), before any of that turn's events.
//! Every reply event carries its `turn`; a new question cancels the old turn,
//! so clients drop audio from older turns. Each turn ends with one
//! `reply_done`, with `interrupted: true` when it was cancelled.

mod asr_worker;
mod pcm;
mod session;
mod tts;
mod turn;

use serde::Deserialize;
use socketioxide::SocketIo;
use socketioxide::extract::TryData;
use socketioxide::handler::ConnectHandler;

/// Register the `/chat` namespace. Clients authenticate with their session
/// token: `io('/chat', { auth: { token } })`.
pub fn register(io: &SocketIo) {
    io.ns("/chat", session::on_connect.with(authenticate));
}

#[derive(Deserialize)]
struct Auth {
    token: String,
}

async fn authenticate(TryData(auth): TryData<Auth>) -> Result<(), &'static str> {
    let Ok(auth) = auth else {
        return Err("unauthorized");
    };
    match crate::auth::validate(&auth.token).await {
        Some(_) => Ok(()),
        None => Err("unauthorized"),
    }
}

/// Stop speaking before exit: no new synthesis, and wait for the running one
/// so onnxruntime / the NPU are idle when the process tears down.
pub async fn shutdown() {
    tts::shutdown().await;
}
