//! MeloTTS front end: text to phone and tone ids through `lexicon.txt`
//! (`word phone... tone...`) and `tokens.txt` (`phone id`). Chinese is looked
//! up per character, English per lower-cased word (spelled letter by letter
//! when the word is missing), punctuation after mapping to its ASCII form.
//! A blank (id 0) goes between and around every phone.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

/// Phone and tone ids of equal length, blanks included.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Phones {
    pub phones: Vec<i32>,
    pub tones: Vec<i32>,
}

pub(crate) struct Lexicon {
    entries: HashMap<String, (Vec<i32>, Vec<i32>)>,
}

/// Punctuation the lexicon spells with its ASCII equivalent.
fn ascii_punct(c: char) -> Option<char> {
    Some(match c {
        '，' | '、' | '；' | '：' | ';' | ':' | '·' => ',',
        '。' | '\n' => '.',
        '！' => '!',
        '？' => '?',
        '“' | '”' | '‘' | '’' | '"' | '（' | '）' | '(' | ')' | '《' | '》' | '【' | '】' | '['
        | ']' | '「' | '」' => '\'',
        '—' | '～' | '~' => '-',
        _ => return None,
    })
}

impl Lexicon {
    pub(crate) fn load(dir: &Path) -> Result<Self> {
        let tokens_path = dir.join("tokens.txt");
        let tokens: HashMap<String, i32> = std::fs::read_to_string(&tokens_path)
            .with_context(|| format!("read {}", tokens_path.display()))?
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                Some((parts.next()?.to_string(), parts.next()?.parse().ok()?))
            })
            .collect();
        let lexicon_path = dir.join("lexicon.txt");
        let text = std::fs::read_to_string(&lexicon_path)
            .with_context(|| format!("read {}", lexicon_path.display()))?;
        Ok(Self::parse(&text, &tokens))
    }

    fn parse(lexicon: &str, tokens: &HashMap<String, i32>) -> Self {
        let mut entries = HashMap::new();
        for line in lexicon.lines() {
            let mut parts = line.split_whitespace();
            let Some(word) = parts.next() else { continue };
            let rest: Vec<&str> = parts.collect();
            let n = rest.len() / 2;
            if n == 0 || !rest.len().is_multiple_of(2) {
                continue;
            }
            let phones: Option<Vec<i32>> =
                rest[..n].iter().map(|p| tokens.get(*p).copied()).collect();
            let tones: Option<Vec<i32>> = rest[n..].iter().map(|t| t.parse().ok()).collect();
            if let (Some(phones), Some(tones)) = (phones, tones) {
                entries.insert(word.to_string(), (phones, tones));
            }
        }
        for p in ['!', '?', '…', ',', '.', '\'', '-'] {
            if let Some(&id) = tokens.get(&p.to_string()) {
                entries.insert(p.to_string(), (vec![id], vec![0]));
            }
        }
        Self { entries }
    }

    /// Ids for `text`. Unknown characters are skipped.
    pub(crate) fn phones(&self, text: &str) -> Phones {
        let text = crate::numbers::normalize(text);
        let mut raw = Phones::default();
        let mut word = String::new();
        for c in text.chars() {
            if c.is_ascii_alphabetic() {
                word.push(c.to_ascii_lowercase());
                continue;
            }
            self.push_word(&mut word, &mut raw);
            if c.is_whitespace() && c != '\n' {
                continue;
            }
            self.push(&ascii_punct(c).unwrap_or(c).to_string(), &mut raw);
        }
        self.push_word(&mut word, &mut raw);

        let mut out = Phones {
            phones: vec![0; raw.phones.len() * 2 + 1],
            tones: vec![0; raw.tones.len() * 2 + 1],
        };
        for (i, (p, t)) in raw.phones.iter().zip(&raw.tones).enumerate() {
            out.phones[2 * i + 1] = *p;
            out.tones[2 * i + 1] = *t;
        }
        out
    }

    fn push_word(&self, word: &mut String, out: &mut Phones) {
        if word.is_empty() {
            return;
        }
        if self.entries.contains_key(word.as_str()) {
            self.push(word, out);
        } else {
            for letter in word.chars() {
                self.push(&letter.to_string(), out);
            }
        }
        word.clear();
    }

    fn push(&self, key: &str, out: &mut Phones) {
        if let Some((phones, tones)) = self.entries.get(key) {
            out.phones.extend_from_slice(phones);
            out.tones.extend_from_slice(tones);
        }
    }
}

#[cfg(test)]
#[path = "melo_test.rs"]
mod melo_test;
