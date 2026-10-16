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
use crate::template::Template;
use crate::{ChatRequest, Delta, FinishReason, LlmConfig, Sampling, Usage};

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
/// Returns the prompt token count. Built and used only on the inference
/// thread, so it need not be `Send`.
pub(crate) trait Backend {
    fn run(&mut self, run: Run<'_>, on_token: &mut dyn FnMut(i32, &str) -> Step) -> Result<u32>;
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
    prompt: String,
    starts_in_reasoning: bool,
    max_new_tokens: i32,
    sampling: Sampling,
    deltas: mpsc::UnboundedSender<Delta>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Clone)]
struct Settings {
    template: Template,
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
        let stop_ids = settings.stop_token_ids.clone();
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
                while let Ok(job) = queue.recv() {
                    thread_shared.busy.send_replace(true);
                    serve(&mut backend, job, &stop_ids, &thread_shared.closing);
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
        let prompt = self
            .settings
            .template
            .render(&req.messages, req.enable_thinking);
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
            prompt: prompt.text,
            starts_in_reasoning: prompt.starts_in_reasoning,
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

fn serve<B: Backend>(backend: &mut B, job: Job, stop_ids: &[i32], closing: &AtomicBool) {
    if job.cancelled.load(Ordering::Relaxed) || closing.load(Ordering::Relaxed) {
        return;
    }
    let mut splitter = ThinkSplitter::new(job.starts_in_reasoning);
    let mut completion_tokens = 0u32;
    let mut stopped = false;
    let result = backend.run(
        Run {
            prompt: &job.prompt,
            max_new_tokens: job.max_new_tokens,
            sampling: &job.sampling,
        },
        &mut |id, text| {
            if job.cancelled.load(Ordering::Relaxed)
                || closing.load(Ordering::Relaxed)
                || job.deltas.is_closed()
            {
                return Step::Stop;
            }
            if stop_ids.contains(&id) {
                stopped = true;
                return Step::Stop;
            }
            completion_tokens += 1;
            for delta in splitter.push(text) {
                let _ = job.deltas.send(delta);
            }
            Step::Continue
        },
    );
    let last = match result {
        Ok(prompt_tokens) => {
            for delta in splitter.finish() {
                let _ = job.deltas.send(delta);
            }
            let limit = u32::try_from(job.max_new_tokens).unwrap_or(0);
            let finish_reason = if !stopped && completion_tokens >= limit {
                FinishReason::Length
            } else {
                FinishReason::Stop
            };
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
