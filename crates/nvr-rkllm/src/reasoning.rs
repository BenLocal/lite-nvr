//! Splits streamed model text into reasoning (`<think>…</think>`) and answer
//! content. Tags may arrive split across tokens, so a partial tag at the end
//! of a chunk is held back until the next one.

use crate::Delta;

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

pub(crate) struct ThinkSplitter {
    reasoning: bool,
    /// Leading whitespace of a section (the newlines around the tags) is
    /// dropped until the section's first visible character.
    section_start: bool,
    pending: String,
}

impl ThinkSplitter {
    pub(crate) fn new(starts_in_reasoning: bool) -> Self {
        Self {
            reasoning: starts_in_reasoning,
            section_start: true,
            pending: String::new(),
        }
    }

    pub(crate) fn push(&mut self, text: &str) -> Vec<Delta> {
        self.pending.push_str(text);
        let mut out = Vec::new();
        loop {
            let tag = if self.reasoning { CLOSE } else { OPEN };
            if let Some(i) = self.pending.find(tag) {
                let before: String = self.pending.drain(..i + tag.len()).collect();
                self.emit(&before[..i], &mut out);
                self.reasoning = !self.reasoning;
                self.section_start = true;
                continue;
            }
            // Hold back a suffix that could still become the tag. Tags are
            // ASCII, so the split point is a char boundary.
            let keep = (1..tag.len())
                .rev()
                .find(|&k| self.pending.ends_with(&tag[..k]))
                .unwrap_or(0);
            let ready: String = self.pending.drain(..self.pending.len() - keep).collect();
            self.emit(&ready, &mut out);
            return out;
        }
    }

    pub(crate) fn finish(&mut self) -> Vec<Delta> {
        let mut out = Vec::new();
        let rest = std::mem::take(&mut self.pending);
        self.emit(&rest, &mut out);
        out
    }

    fn emit(&mut self, text: &str, out: &mut Vec<Delta>) {
        let text = if self.section_start {
            text.trim_start()
        } else {
            text
        };
        if text.is_empty() {
            return;
        }
        self.section_start = false;
        out.push(if self.reasoning {
            Delta::Reasoning(text.to_string())
        } else {
            Delta::Content(text.to_string())
        });
    }
}

#[cfg(test)]
#[path = "reasoning_test.rs"]
mod reasoning_test;
