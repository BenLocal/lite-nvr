//! On-board LLM inference through the Rockchip RKLLM runtime.
//!
//! [`Engine::spawn`] loads a `.rkllm` model in an isolated worker process
//! and serves [`ChatRequest`]s one at a time (the NPU runs one generation at
//! a time); each request streams [`Delta`]s through a [`Generation`].
//! `librkllmrt.so` is dlopened at run time, so the crate builds and its tests
//! run on any host.
//!
//! The host binary must call [`run_worker_if_requested`] before initializing
//! its services. Workers use the same executable and a private socket, so
//! runtime logs on stdout/stderr cannot corrupt the protocol.
//!
//! Runtime 1.3.0 caveats this crate works around:
//! - a failed `rkllm_init` may double free; only the worker dies, and a failed
//!   load is never automatically retried;
//! - `rkllm_destroy` sometimes hangs in an NPU ioctl, so the handle is never
//!   destroyed; the kernel reclaims NPU memory when the process exits.

use std::path::PathBuf;

mod engine;
mod ffi;
mod process;
mod reasoning;
mod rkllm;
mod template;

pub use engine::{Engine, EngineState, GenerateError, Generation};
pub use process::run_worker_if_requested;
pub use template::{Prompt, Template};

/// Qwen's `<|endoftext|>` and `<|im_end|>`.
pub const DEFAULT_STOP_TOKEN_IDS: [i32; 2] = [151643, 151645];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmConfig {
    /// `.rkllm` model (RK3588 needs a w8a8-family quantization).
    pub model: PathBuf,
    /// `librkllmrt.so` path, or a bare name searched on `LD_LIBRARY_PATH`.
    pub lib: String,
    pub template: Template,
    pub max_context_len: i32,
    /// Default per-request cap; a request's `max_tokens` may lower it.
    pub max_new_tokens: i32,
    /// Generation stops (and is aborted) on any of these token ids.
    pub stop_token_ids: Vec<i32>,
    /// For conversions whose EOS id is wrong; rely on `stop_token_ids` instead.
    pub ignore_eos: bool,
    /// Requests that may wait behind the running one before `Busy`.
    pub queue_depth: usize,
}

impl LlmConfig {
    pub fn new(model: impl Into<PathBuf>, template: Template) -> Self {
        Self {
            model: model.into(),
            lib: "librkllmrt.so".into(),
            template,
            max_context_len: 4096,
            max_new_tokens: 512,
            stop_token_ids: DEFAULT_STOP_TOKEN_IDS.to_vec(),
            ignore_eos: false,
            queue_depth: 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Per-request sampling overrides; `None` keeps the engine default.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Sampling {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<i32>,
    pub repetition_penalty: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
    pub sampling: Sampling,
    pub enable_thinking: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    /// A stop token or the model's own end of sequence.
    Stop,
    /// Reached the token limit.
    Length,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    /// Text inside `<think>…</think>`.
    Reasoning(String),
    /// Answer text.
    Content(String),
    /// The last item of a successful generation.
    Done {
        usage: Usage,
        finish_reason: FinishReason,
    },
    /// The runtime failed; nothing follows.
    Error(String),
}
