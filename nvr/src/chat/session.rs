//! One `/chat` connection: a task that owns the conversation and reacts to
//! client events, recognizer results and finished turns, in order.

use std::sync::LazyLock;

use bytes::Bytes;
use nvr_asr::Transcript;
use nvr_rkllm::Message;
use serde_json::json;
use socketioxide::extract::{Data, SocketRef};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

use super::asr_worker::AsrWorker;
use super::pcm::s16le_to_f32;

/// Turns (question + answer) kept in the prompt.
const MAX_TURNS: usize = 8;
/// History budget in characters, roughly one token each for Chinese; keeps
/// the prompt well inside the default 4096-token context.
const MAX_HISTORY_CHARS: usize = 2000;

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    std::env::var("NVR_CHAT_SYSTEM_PROMPT").unwrap_or_else(|_| {
        "你是一个语音助手。回答要简短、口语化，适合直接朗读；\
         不要使用 Markdown、列表、代码或表情符号。"
            .into()
    })
});

pub(super) enum Event {
    Start,
    Audio(Vec<f32>),
    Stop,
    Text(String),
    Cancel,
    Reset,
    /// A recognizer result for push-to-talk round `epoch`.
    Asr {
        epoch: u64,
        t: Transcript,
    },
    /// The recognizer has emitted everything for round `epoch`.
    Flushed {
        epoch: u64,
    },
    /// A turn finished without being cancelled.
    TurnDone {
        turn: u64,
        question: String,
        reply: String,
    },
}

pub(super) async fn on_connect(socket: SocketRef) {
    let (tx, rx) = unbounded_channel();
    // Enqueue synchronously: async Socket.IO handlers are spawned separately
    // and can reorder cancel/text or start/audio/stop before the session sees them.
    let on = |name: &'static str, make: fn() -> Event| {
        let tx = tx.clone();
        socket.on(name, move || {
            let _ = tx.send(make());
            std::future::ready(())
        });
    };
    on("start", || Event::Start);
    on("stop", || Event::Stop);
    on("cancel", || Event::Cancel);
    on("reset", || Event::Reset);
    let audio = tx.clone();
    socket.on("audio", move |Data::<Bytes>(pcm)| {
        let _ = audio.send(Event::Audio(s16le_to_f32(&pcm)));
        std::future::ready(())
    });
    let text = tx.clone();
    socket.on("text", move |Data::<String>(question)| {
        let _ = text.send(Event::Text(question));
        std::future::ready(())
    });
    let closed = CancellationToken::new();
    let on_close = closed.clone();
    socket.on_disconnect(async move || on_close.cancel());

    // Load the models while the user gets ready to speak.
    tokio::spawn(async {
        if let Some(hub) = crate::asr::hub::AsrHub::get() {
            let _ = hub.models().await;
        }
        let _ = super::tts::get().await;
    });

    let session = Session {
        socket,
        events: tx,
        asr: None,
        epoch: 0,
        listening: false,
        heard: Vec::new(),
        history: Vec::new(),
        turn: None,
        next_turn: 0,
    };
    tokio::spawn(session.run(rx, closed));
}

struct Session {
    socket: SocketRef,
    /// Handed to the recognizer and to turns so they can report back.
    events: UnboundedSender<Event>,
    asr: Option<AsrWorker>,
    /// Push-to-talk round; recognizer results from older rounds are ignored.
    epoch: u64,
    listening: bool,
    /// Final segments of the current round.
    heard: Vec<String>,
    history: Vec<Message>,
    /// The running turn's id and its cancel token.
    turn: Option<(u64, CancellationToken)>,
    next_turn: u64,
}

impl Session {
    async fn run(mut self, mut rx: UnboundedReceiver<Event>, closed: CancellationToken) {
        loop {
            let event = tokio::select! {
                _ = closed.cancelled() => break,
                event = rx.recv() => match event {
                    Some(event) => event,
                    None => break,
                },
            };
            self.handle(event).await;
        }
        self.cancel_turn();
    }

    async fn handle(&mut self, event: Event) {
        match event {
            Event::Start => self.start_listening().await,
            Event::Audio(samples) => {
                if self.listening
                    && let Some(asr) = &self.asr
                {
                    asr.audio(samples);
                }
            }
            Event::Stop => {
                if self.listening
                    && let Some(asr) = &self.asr
                {
                    self.listening = false;
                    asr.flush();
                }
            }
            Event::Text(question) => self.start_turn(question),
            Event::Cancel => self.cancel_turn(),
            Event::Reset => {
                self.cancel_turn();
                self.history.clear();
            }
            Event::Asr { epoch, t } if epoch == self.epoch => match t {
                Transcript::Partial { text } => {
                    let text = format!("{}{text}", self.heard.concat());
                    self.emit("asr_partial", json!({ "text": text }));
                }
                Transcript::Final { text, .. } => self.heard.push(text),
            },
            Event::Flushed { epoch } if epoch == self.epoch => {
                let question = std::mem::take(&mut self.heard).concat();
                // An empty `asr_final` tells the client no turn follows.
                self.emit("asr_final", json!({ "text": question }));
                self.start_turn(question);
            }
            Event::Asr { .. } | Event::Flushed { .. } => {}
            Event::TurnDone {
                turn,
                question,
                reply,
            } => {
                if self.turn.as_ref().is_some_and(|(id, _)| *id == turn) {
                    self.turn = None;
                    self.history.push(Message::user(question));
                    self.history.push(Message::assistant(reply));
                    trim_history(&mut self.history, MAX_TURNS, MAX_HISTORY_CHARS);
                }
            }
        }
    }

    /// Push to talk: interrupts the current reply and starts a new round.
    async fn start_listening(&mut self) {
        self.cancel_turn();
        if self.asr.is_none() {
            match self.spawn_asr().await {
                Ok(asr) => self.asr = Some(asr),
                Err(err) => {
                    self.emit(
                        "error",
                        json!({ "message": format!("ASR unavailable: {err:#}") }),
                    );
                    return;
                }
            }
        }
        self.epoch += 1;
        self.heard.clear();
        self.listening = true;
        if let Some(asr) = &self.asr {
            asr.reset(self.epoch);
        }
    }

    async fn spawn_asr(&self) -> anyhow::Result<AsrWorker> {
        let hub =
            crate::asr::hub::AsrHub::get().ok_or_else(|| anyhow::anyhow!("asr not initialized"))?;
        let models = hub.models().await?;
        AsrWorker::spawn(models, self.events.clone())
    }

    fn start_turn(&mut self, question: String) {
        self.cancel_turn();
        let question = question.trim().to_string();
        if question.is_empty() {
            return;
        }
        self.next_turn += 1;
        let turn = self.next_turn;
        let cancel = CancellationToken::new();
        let mut messages = Vec::with_capacity(self.history.len() + 2);
        messages.push(Message::system(SYSTEM_PROMPT.as_str()));
        messages.extend(self.history.iter().cloned());
        messages.push(Message::user(question.clone()));
        tokio::spawn(super::turn::run(
            self.socket.clone(),
            turn,
            question,
            messages,
            cancel.clone(),
            self.events.clone(),
        ));
        self.turn = Some((turn, cancel));
    }

    /// An interrupted turn is left out of the history.
    fn cancel_turn(&mut self) {
        if let Some((_, cancel)) = self.turn.take() {
            cancel.cancel();
        }
    }

    fn emit(&self, event: &'static str, data: serde_json::Value) {
        let _ = self.socket.emit(event, &data);
    }
}

/// Once the history outgrows `max_turns` or `max_chars`, keep only the latest
/// turn (or nothing, if that alone is too long). Dropping turns changes the
/// prompt's start, so the LLM must prefill the whole prompt again instead of
/// continuing from its KV cache; trimming rarely and deeply keeps that rare
/// and short. `history` alternates user / assistant messages.
pub(super) fn trim_history(history: &mut Vec<Message>, max_turns: usize, max_chars: usize) {
    let chars = |h: &[Message]| h.iter().map(|m| m.content.chars().count()).sum::<usize>();
    if history.len() <= max_turns * 2 && chars(history) <= max_chars {
        return;
    }
    history.drain(..history.len().saturating_sub(2));
    if chars(history) > max_chars {
        history.clear();
    }
}

#[cfg(test)]
#[path = "session_test.rs"]
mod session_test;
