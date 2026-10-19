//! The inference thread and its request queue.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::task::{Context, Poll};

use anyhow::Result;
use futures::Stream;
use tokio::sync::{mpsc, watch};

use crate::reasoning::ThinkSplitter;
use crate::template::{Prompt, Template};
use crate::{ChatRequest, Delta, FinishReason, LlmConfig, Message, Sampling, Usage};

/// What the backend's token callback tells it to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Continue,
    Stop,
}

pub(crate) struct Run<'a> {
    pub prompt: &'a str,
    pub max_new_tokens: i32,
    pub sampling: &'a Sampling,
}

/// One loaded model. `run` blocks until generation ends; `on_token` sees every
/// generated token (with empty text while a UTF-8 character is incomplete).
/// The prompt and the generated tokens stay in the KV cache, so the next run
/// continues the conversation until `clear_history`. Returns the number of
/// tokens prefilled. Built and used only on the inference thread, so it need
/// not be `Send`.
pub(crate) trait Backend {
    fn run(&mut self, run: Run<'_>, on_token: &mut dyn FnMut(i32, &str) -> Step) -> Result<u32>;
    fn clear_history(&mut self) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineState {
    Loading,
    Ready,
    /// Load failed or the inference thread is gone; a restart is needed.
    Failed(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum GenerateError {
    NotReady(EngineState),
    /// The running request plus `queue_depth` waiting ones are taken.
    Busy,
}

impl std::fmt::Display for GenerateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotReady(EngineState::Loading) => f.write_str("model is still loading"),
            Self::NotReady(EngineState::Failed(reason)) => write!(f, "model unavailable: {reason}"),
            Self::NotReady(EngineState::Ready) => f.write_str("model unavailable"),
            Self::Busy => f.write_str("model is busy"),
        }
    }
}

impl std::error::Error for GenerateError {}

struct Job {
    messages: Vec<Message>,
    enable_thinking: bool,
    max_new_tokens: i32,
    sampling: Sampling,
    deltas: mpsc::UnboundedSender<Delta>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone)]
struct Settings {
    template: Template,
    max_context_len: i32,
    max_new_tokens: i32,
    stop_token_ids: Vec<i32>,
    queue_depth: usize,
}

/// State shared with the inference thread for shutdown.
struct Shared {
    closing: AtomicBool,
    /// True while the backend is running a request.
    busy: watch::Sender<bool>,
}

/// Handle to the inference thread. Cheap to clone; the thread exits once every
/// handle is dropped and the queue drains.
#[derive(Clone)]
pub struct Engine {
    jobs: SyncSender<Job>,
    state: watch::Receiver<EngineState>,
    settings: Arc<Settings>,
    shared: Arc<Shared>,
}

impl Engine {
    /// Start the inference thread and load `cfg.model` on it. Returns at once
    /// in [`EngineState::Loading`]; see [`Engine::wait_ready`].
    pub fn spawn(cfg: LlmConfig) -> Engine {
        let settings = Settings {
            template: cfg.template,
            max_context_len: cfg.max_context_len,
            max_new_tokens: cfg.max_new_tokens,
            stop_token_ids: cfg.stop_token_ids.clone(),
            queue_depth: cfg.queue_depth,
        };
        Self::spawn_with(settings, move || crate::rkllm::RkllmBackend::load(&cfg))
    }

    fn spawn_with<B, F>(settings: Settings, load: F) -> Engine
    where
        B: Backend,
        F: FnOnce() -> Result<B> + Send + 'static,
    {
        let (jobs, queue) = sync_channel::<Job>(settings.queue_depth);
        let (state_tx, state) = watch::channel(EngineState::Loading);
        let thread_settings = settings.clone();
        let shared = Arc::new(Shared {
            closing: AtomicBool::new(false),
            busy: watch::channel(false).0,
        });
        let thread_shared = Arc::clone(&shared);
        let spawned = std::thread::Builder::new()
            .name("rkllm".into())
            .spawn(move || {
                let mut backend = match load() {
                    Ok(backend) => backend,
                    Err(err) => {
                        log::error!("rkllm: model load failed: {err:#}");
                        let _ = state_tx.send(EngineState::Failed(format!("{err:#}")));
                        return;
                    }
                };
                let _ = state_tx.send(EngineState::Ready);
                let mut cached = None;
                while let Ok(job) = queue.recv() {
                    thread_shared.busy.send_replace(true);
                    serve(
                        &mut backend,
                        job,
                        &thread_settings,
                        &mut cached,
                        &thread_shared.closing,
                    );
                    thread_shared.busy.send_replace(false);
                }
            });
        if let Err(err) = spawned {
            let (failed_tx, failed) = watch::channel(EngineState::Failed(format!(
                "cannot start inference thread: {err}"
            )));
            drop(failed_tx);
            return Engine {
                jobs,
                state: failed,
                settings: Arc::new(settings),
                shared,
            };
        }
        Engine {
            jobs,
            state,
            settings: Arc::new(settings),
            shared,
        }
    }

    /// Stop accepting requests, end the running one at its next token and
    /// wait up to `timeout` for the runtime to go idle, so the process can
    /// exit without the runtime mid-inference. Queued requests are dropped.
    pub async fn shutdown(&self, timeout: std::time::Duration) {
        self.shared.closing.store(true, Ordering::Relaxed);
        let mut busy = self.shared.busy.subscribe();
        if tokio::time::timeout(timeout, busy.wait_for(|b| !*b))
            .await
            .is_err()
        {
            log::warn!("rkllm: still generating after {timeout:?}; exiting anyway");
        }
    }

    pub fn state(&self) -> EngineState {
        self.state.borrow().clone()
    }

    /// Wait for the load to finish; errors with the reason if it failed.
    pub async fn wait_ready(&self) -> Result<()> {
        let mut state = self.state.clone();
        let ready = state
            .wait_for(|s| *s != EngineState::Loading)
            .await
            .map(|s| s.clone())
            .unwrap_or_else(|_| self.state());
        match ready {
            EngineState::Ready => Ok(()),
            EngineState::Failed(reason) => anyhow::bail!(reason),
            EngineState::Loading => anyhow::bail!("inference thread exited while loading"),
        }
    }

    pub fn template(&self) -> Template {
        self.settings.template
    }

    /// Queue a request. Dropping the returned [`Generation`] cancels it.
    pub fn generate(&self, req: ChatRequest) -> Result<Generation, GenerateError> {
        match self.state() {
            EngineState::Ready => {}
            other => return Err(GenerateError::NotReady(other)),
        }
        if self.shared.closing.load(Ordering::Relaxed) {
            return Err(GenerateError::NotReady(EngineState::Failed(
                "shutting down".into(),
            )));
        }
        let max_new_tokens = req
            .max_tokens
            .map_or(self.settings.max_new_tokens, |n| {
                i32::try_from(n)
                    .unwrap_or(i32::MAX)
                    .min(self.settings.max_new_tokens)
            })
            .max(1);
        let (deltas, rx) = mpsc::unbounded_channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let job = Job {
            messages: req.messages,
            enable_thinking: req.enable_thinking,
            max_new_tokens,
            sampling: req.sampling,
            deltas,
            cancelled: Arc::clone(&cancelled),
        };
        match self.jobs.try_send(job) {
            Ok(()) => Ok(Generation { rx, cancelled }),
            Err(TrySendError::Full(_)) => Err(GenerateError::Busy),
            Err(TrySendError::Disconnected(_)) => Err(GenerateError::NotReady(
                EngineState::Failed("inference thread stopped".into()),
            )),
        }
    }
}

/// The conversation whose tokens are in the KV cache: the request's messages
/// plus the reply generated for them.
struct Cached {
    messages: Vec<Message>,
    tokens: u32,
}

/// The new messages of a request that extends the cached conversation.
fn continuation<'a>(cached: &Cached, messages: &'a [Message]) -> Option<&'a [Message]> {
    let n = cached.messages.len();
    (messages.len() > n && messages[..n] == cached.messages[..]).then(|| &messages[n..])
}

/// Prefill is the slow part on the NPU (about 30 tokens/s for a 0.8B model on
/// RK3588), so a request that extends the cached conversation only prefills
/// its new messages. Anything else clears the cache and prefills everything.
/// Returns the prompt and the number of tokens it continues from.
fn prepare<B: Backend>(
    backend: &mut B,
    job: &Job,
    settings: &Settings,
    cached: Option<Cached>,
) -> Result<(Prompt, u32)> {
    if let Some(c) = &cached
        && let Some(new) = continuation(c, &job.messages)
    {
        let prompt = settings
            .template
            .render_continuation(new, job.enable_thinking);
        // Characters over-count tokens, which errs towards a fresh prefill.
        let needed = u64::from(c.tokens)
            + prompt.text.chars().count() as u64
            + u64::try_from(job.max_new_tokens).unwrap_or(0);
        if needed <= u64::try_from(settings.max_context_len).unwrap_or(0) {
            return Ok((prompt, c.tokens));
        }
    }
    backend.clear_history()?;
    let prompt = settings.template.render(&job.messages, job.enable_thinking);
    Ok((prompt, 0))
}

fn serve<B: Backend>(
    backend: &mut B,
    job: Job,
    settings: &Settings,
    cached: &mut Option<Cached>,
    closing: &AtomicBool,
) {
    if job.cancelled.load(Ordering::Relaxed) || closing.load(Ordering::Relaxed) {
        return;
    }
    // Whatever happens below, the cache only describes this run's outcome.
    let (prompt, cached_tokens) = match prepare(backend, &job, settings, cached.take()) {
        Ok(prepared) => prepared,
        Err(err) => {
            log::warn!("rkllm: clearing the KV cache failed: {err:#}");
            let _ = job.deltas.send(Delta::Error(format!("{err:#}")));
            return;
        }
    };
    let mut splitter = ThinkSplitter::new(prompt.starts_in_reasoning);
    let mut completion_tokens = 0u32;
    let mut stopped = false;
    let mut cancelled = false;
    let mut reply = String::new();
    let result = backend.run(
        Run {
            prompt: &prompt.text,
            max_new_tokens: job.max_new_tokens,
            sampling: &job.sampling,
        },
        &mut |id, text| {
            if job.cancelled.load(Ordering::Relaxed)
                || closing.load(Ordering::Relaxed)
                || job.deltas.is_closed()
            {
                cancelled = true;
                return Step::Stop;
            }
            if settings.stop_token_ids.contains(&id) {
                stopped = true;
                return Step::Stop;
            }
            completion_tokens += 1;
            for delta in splitter.push(text) {
                if let Delta::Content(text) = &delta {
                    reply.push_str(text);
                }
                let _ = job.deltas.send(delta);
            }
            Step::Continue
        },
    );
    let last = match result {
        Ok(prefilled) => {
            for delta in splitter.finish() {
                if let Delta::Content(text) = &delta {
                    reply.push_str(text);
                }
                let _ = job.deltas.send(delta);
            }
            let prompt_tokens = cached_tokens + prefilled;
            log::debug!("rkllm: prefilled {prefilled} tokens after {cached_tokens} cached");
            let limit = u32::try_from(job.max_new_tokens).unwrap_or(0);
            let finish_reason = if !stopped && completion_tokens >= limit {
                FinishReason::Length
            } else {
                FinishReason::Stop
            };
            // A reply that ended on a stop token or the model's own end of
            // sequence is in the cache as generated, without that end token.
            // Cancelled or truncated runs leave the cache for the next request
            // to clear.
            if !cancelled && finish_reason == FinishReason::Stop {
                let mut messages = job.messages;
                messages.push(Message::assistant(reply));
                *cached = Some(Cached {
                    messages,
                    tokens: prompt_tokens + completion_tokens,
                });
            }
            Delta::Done {
                usage: Usage {
                    prompt_tokens,
                    completion_tokens,
                },
                finish_reason,
            }
        }
        Err(err) => {
            log::warn!("rkllm: generation failed: {err:#}");
            Delta::Error(format!("{err:#}"))
        }
    };
    let _ = job.deltas.send(last);
}

/// A queued or running request's output. Ends after [`Delta::Done`] or
/// [`Delta::Error`]; dropping it stops the generation at the next token.
pub struct Generation {
    rx: mpsc::UnboundedReceiver<Delta>,
    cancelled: Arc<AtomicBool>,
}

impl Stream for Generation {
    type Item = Delta;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Delta>> {
        self.rx.poll_recv(cx)
    }
}

impl Drop for Generation {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
#[path = "engine_test.rs"]
mod engine_test;
