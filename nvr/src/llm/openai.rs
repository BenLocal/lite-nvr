//! The OpenAI chat-completions subset served at `/api/llm/v1`: request
//! validation and response / stream-chunk shapes. Responses follow OpenAI's
//! format rather than nvr's `BaseResponse`, so standard clients work as is.

use nvr_rkllm::{ChatRequest, Delta, FinishReason, Message, Sampling, Usage};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionRequest {
    pub messages: Vec<InMessage>,
    #[serde(default)]
    pub stream: bool,
    pub stream_options: Option<StreamOptions>,
    pub max_tokens: Option<u32>,
    pub max_completion_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<i32>,
    pub repetition_penalty: Option<f32>,
    pub chat_template_kwargs: Option<TemplateKwargs>,
    // Unsupported; rejected when present.
    pub stop: Option<Value>,
    pub tools: Option<Value>,
    pub n: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamOptions {
    #[serde(default)]
    pub include_usage: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TemplateKwargs {
    #[serde(default)]
    pub enable_thinking: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct InMessage {
    pub role: String,
    pub content: Content,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Content {
    Text(String),
    Parts(Vec<Part>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct Part {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
}

impl CompletionRequest {
    pub(crate) fn include_usage(&self) -> bool {
        self.stream_options
            .as_ref()
            .is_some_and(|o| o.include_usage)
    }

    /// Validate and convert; the error is the 400 message.
    pub(crate) fn into_chat(self) -> Result<ChatRequest, String> {
        if self.stop.as_ref().is_some_and(|v| !v.is_null()) {
            return Err("stop is not supported".into());
        }
        if self.tools.as_ref().is_some_and(|v| !v.is_null()) {
            return Err("tools are not supported".into());
        }
        if self.n.is_some_and(|n| n != 1) {
            return Err("only n = 1 is supported".into());
        }
        if self.messages.is_empty() {
            return Err("messages must not be empty".into());
        }
        let messages = self
            .messages
            .into_iter()
            .map(|m| {
                let content = match m.content {
                    Content::Text(text) => text,
                    Content::Parts(parts) => parts
                        .into_iter()
                        .map(|p| match (p.kind.as_str(), p.text) {
                            ("text", Some(text)) => Ok(text),
                            _ => Err("only text content is supported".to_string()),
                        })
                        .collect::<Result<Vec<_>, _>>()?
                        .join("\n"),
                };
                match m.role.as_str() {
                    "system" | "developer" => Ok(Message::system(content)),
                    "user" => Ok(Message::user(content)),
                    "assistant" => Ok(Message::assistant(content)),
                    other => Err(format!("unsupported role {other:?}")),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ChatRequest {
            messages,
            max_tokens: self.max_completion_tokens.or(self.max_tokens),
            sampling: Sampling {
                temperature: self.temperature,
                top_p: self.top_p,
                top_k: self.top_k,
                repetition_penalty: self.repetition_penalty,
            },
            enable_thinking: self.chat_template_kwargs.is_some_and(|k| k.enable_thinking),
        })
    }
}

pub(crate) fn error_body(message: &str, kind: &str) -> Value {
    json!({ "error": { "message": message, "type": kind, "code": null } })
}

fn finish_reason(reason: FinishReason) -> &'static str {
    match reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
    }
}

fn usage_json(usage: Usage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.prompt_tokens + usage.completion_tokens,
    })
}

/// Identifies one completion across its response or stream chunks.
pub(crate) struct Completion {
    pub id: String,
    pub created: i64,
    pub model: String,
}

impl Completion {
    pub(crate) fn new(model: &str) -> Self {
        Self {
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
            created: chrono::Utc::now().timestamp(),
            model: model.to_string(),
        }
    }

    /// The non-streaming response for a finished generation, or the error
    /// message if it failed.
    pub(crate) fn response(&self, deltas: Vec<Delta>) -> Result<Value, String> {
        let (mut content, mut reasoning) = (String::new(), String::new());
        let mut done = None;
        for delta in deltas {
            match delta {
                Delta::Content(t) => content.push_str(&t),
                Delta::Reasoning(t) => reasoning.push_str(&t),
                Delta::Done {
                    usage,
                    finish_reason,
                } => done = Some((usage, finish_reason)),
                Delta::Error(e) => return Err(e),
            }
        }
        let (usage, reason) = done.ok_or("generation ended unexpectedly")?;
        let mut message = json!({ "role": "assistant", "content": content });
        if !reasoning.is_empty() {
            message["reasoning_content"] = json!(reasoning);
        }
        Ok(json!({
            "id": self.id,
            "object": "chat.completion",
            "created": self.created,
            "model": self.model,
            "choices": [{ "index": 0, "message": message, "finish_reason": finish_reason(reason) }],
            "usage": usage_json(usage),
        }))
    }

    fn chunk(&self, delta: Value, finish: Option<&str>) -> Value {
        json!({
            "id": self.id,
            "object": "chat.completion.chunk",
            "created": self.created,
            "model": self.model,
            "choices": [{ "index": 0, "delta": delta, "finish_reason": finish }],
        })
    }

    /// The opening chunk that announces the assistant role.
    pub(crate) fn role_chunk(&self) -> Value {
        self.chunk(json!({ "role": "assistant" }), None)
    }

    /// Stream chunks (SSE `data:` payloads) for one delta.
    pub(crate) fn chunks(&self, delta: Delta, include_usage: bool) -> Vec<Value> {
        match delta {
            Delta::Content(t) => vec![self.chunk(json!({ "content": t }), None)],
            Delta::Reasoning(t) => vec![self.chunk(json!({ "reasoning_content": t }), None)],
            Delta::Done {
                usage,
                finish_reason: reason,
            } => {
                let mut out = vec![self.chunk(json!({}), Some(finish_reason(reason)))];
                if include_usage {
                    out.push(json!({
                        "id": self.id,
                        "object": "chat.completion.chunk",
                        "created": self.created,
                        "model": self.model,
                        "choices": [],
                        "usage": usage_json(usage),
                    }));
                }
                out
            }
            Delta::Error(e) => vec![error_body(&e, "server_error")],
        }
    }
}

#[cfg(test)]
#[path = "openai_test.rs"]
mod openai_test;
