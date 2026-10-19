//! Chat templates. The runtime's own template is disabled after loading, so
//! every request renders the full conversation here.

use crate::{Message, Role};

/// Conversation format; must match the model family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// ChatML, used by Qwen2 / 2.5 / 3 / 3.5.
    Qwen,
    /// DeepSeek-R1 distills.
    DeepseekR1,
}

impl std::str::FromStr for Template {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s.trim() {
            "qwen" => Ok(Self::Qwen),
            "deepseek-r1" => Ok(Self::DeepseekR1),
            other => anyhow::bail!("unknown template {other:?} (expected qwen or deepseek-r1)"),
        }
    }
}

/// A prompt ready for the runtime, plus whether the model's output starts
/// inside a `<think>` section.
#[derive(Debug, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    pub starts_in_reasoning: bool,
}

/// Closes the reasoning section up front, which is how both families turn
/// thinking off.
const EMPTY_THINK: &str = "<think>\n\n</think>\n\n";

impl Template {
    pub fn render(self, messages: &[Message], enable_thinking: bool) -> Prompt {
        let mut text = String::new();
        match self {
            Self::Qwen => {
                for m in messages {
                    let role = match m.role {
                        Role::System => "system",
                        Role::User => "user",
                        Role::Assistant => "assistant",
                    };
                    text.push_str(&format!("<|im_start|>{role}\n{}<|im_end|>\n", m.content));
                }
                text.push_str("<|im_start|>assistant\n");
                if !enable_thinking {
                    text.push_str(EMPTY_THINK);
                }
                Prompt {
                    text,
                    starts_in_reasoning: false,
                }
            }
            Self::DeepseekR1 => {
                for m in messages {
                    match m.role {
                        Role::System => text.push_str(&m.content),
                        Role::User => text.push_str(&format!("<｜User｜>{}", m.content)),
                        Role::Assistant => text.push_str(&format!(
                            "<｜Assistant｜>{}<｜end▁of▁sentence｜>",
                            m.content
                        )),
                    }
                }
                text.push_str("<｜Assistant｜>");
                text.push_str(if enable_thinking {
                    "<think>\n"
                } else {
                    EMPTY_THINK
                });
                Prompt {
                    text,
                    starts_in_reasoning: enable_thinking,
                }
            }
        }
    }

    /// Continue a conversation whose earlier messages, ending with the
    /// model's own reply, are already in the runtime's KV cache: close that
    /// reply (generation stopped before its end token) and render only `new`.
    pub fn render_continuation(self, new: &[Message], enable_thinking: bool) -> Prompt {
        let end = match self {
            Self::Qwen => "<|im_end|>\n",
            Self::DeepseekR1 => "<｜end▁of▁sentence｜>",
        };
        let rest = self.render(new, enable_thinking);
        Prompt {
            text: format!("{end}{}", rest.text),
            starts_in_reasoning: rest.starts_in_reasoning,
        }
    }
}

#[cfg(test)]
#[path = "template_test.rs"]
mod template_test;
