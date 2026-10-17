//! Text preparation for speech: split streamed text into sentences and strip
//! what should not be read aloud (Markdown, links, emoji, control characters).

/// Clause-ending punctuation; the sentence ends right after it.
const TERMINATORS: &[char] = &['。', '！', '？', '；', '!', '?', ';', '…', '\n'];
/// Closing marks that stay with the sentence they end (`好！”`).
const CLOSERS: &[char] = &['”', '’', '」', '』', '）', ')', '"', '\''];
/// Pauses where an overlong sentence may be cut.
const SOFT_BREAKS: &[char] = &['，', ',', '、', '：', ':'];
/// Past this many characters, cut at the last soft break so speech starts
/// without waiting for the full stop.
const SOFT_LIMIT: usize = 40;

/// Collects streamed text and hands out complete sentences.
#[derive(Default)]
pub struct SentenceSplitter {
    buf: String,
}

impl SentenceSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add text; returns the sentences it completed, cleaned for speech.
    /// Sentences with nothing speakable are dropped.
    pub fn push(&mut self, text: &str) -> Vec<String> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        while let Some(end) = self.sentence_end() {
            let sentence: String = self.buf.drain(..end).collect();
            emit(&sentence, &mut out);
        }
        out
    }

    /// The text left after the stream ended.
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buf);
        let mut out = Vec::new();
        emit(&rest, &mut out);
        out.pop()
    }

    /// Byte offset just past the first complete sentence, if any.
    fn sentence_end(&self) -> Option<usize> {
        let mut chars = self.buf.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            let after = i + c.len_utf8();
            let ends = if TERMINATORS.contains(&c) {
                true
            } else if c == '.' {
                // "3.14" and "e.g" continue; ". " ends. At the end of the
                // buffer, wait for the next character to decide.
                match chars.peek() {
                    Some((_, next)) => next.is_whitespace(),
                    None => return None,
                }
            } else {
                false
            };
            if ends {
                let mut end = after;
                while let Some(&(j, next)) = chars.peek() {
                    if CLOSERS.contains(&next) || TERMINATORS.contains(&next) {
                        end = j + next.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                return Some(end);
            }
        }
        if self.buf.chars().count() > SOFT_LIMIT {
            return self
                .buf
                .char_indices()
                .rfind(|(_, c)| SOFT_BREAKS.contains(c))
                .map(|(i, c)| i + c.len_utf8());
        }
        None
    }
}

fn emit(sentence: &str, out: &mut Vec<String>) {
    let clean = clean(sentence);
    if clean.chars().any(char::is_alphanumeric) {
        out.push(clean);
    }
}

/// Strip what should not be read aloud and collapse whitespace.
pub fn clean(text: &str) -> String {
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("```") {
            continue;
        }
        let line = line.trim_start_matches(['#', '>']).trim_start();
        let line = strip_list_marker(line);
        lines.push(strip_inline(line));
    }
    let joined = lines.join(" ");
    joined.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_list_marker(line: &str) -> &str {
    if let Some(rest) = line.strip_prefix(['-', '*', '+'])
        && rest.starts_with(' ')
    {
        return rest.trim_start();
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(rest) = line[digits..].strip_prefix(['.', ')'])
        && rest.starts_with(' ')
    {
        return rest.trim_start();
    }
    line
}

fn strip_inline(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        // [text](url) keeps the text.
        if c == '['
            && let Some(close) = rest.find("](")
            && let Some(paren) = rest[close..].find(')')
        {
            out.push_str(&rest[1..close]);
            rest = &rest[close + paren + 1..];
            continue;
        }
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
            rest = &rest[len..];
            continue;
        }
        rest = &rest[c.len_utf8()..];
        if matches!(c, '*' | '_' | '`' | '~' | '|') || is_emoji(c) {
            continue;
        }
        out.push(if c.is_control() { ' ' } else { c });
    }
    out
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE00..=0xFE0F | 0x200D)
}

#[cfg(test)]
#[path = "text_test.rs"]
mod text_test;
