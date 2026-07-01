//! Opaque GB code + national-standard SSRC generator.

use std::sync::atomic::{AtomicU16, Ordering};

/// A GB device/channel code. Stored opaque (never rejected on malformed input);
/// `parse()` is a best-effort structural view.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GbCode(pub String);

impl GbCode {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Best-effort structural parse. Returns None if it doesn't look like a
    /// 20-digit code (callers must still accept the raw code regardless).
    pub fn parse(&self) -> Option<GbCodeParts> {
        let s = &self.0;
        if s.len() != 20 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(GbCodeParts {
            region: s[0..8].to_string(),
            industry: s[8..10].to_string(),
            type_code: s[10..13].to_string(),
            sequence: s[13..20].to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GbCodeParts {
    pub region: String,
    pub industry: String,
    pub type_code: String,
    pub sequence: String,
}

/// GB-format SSRC generator seeded from the platform id's domain (first 10 digits).
pub struct SsrcGenerator {
    domain5: String, // digits [3..8] of the platform id, used as the SSRC middle
    seq: AtomicU16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsrcKind {
    Live,
    Playback,
}

impl SsrcGenerator {
    /// `platform_id` is the server's 20-digit GB code. Falls back to "00000"
    /// middle if it can't extract 5 digits (still produces a valid 10-digit SSRC).
    pub fn new(platform_id: &str) -> Self {
        let domain5 = platform_id
            .get(3..8)
            .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
            .unwrap_or("00000")
            .to_string();
        Self {
            domain5,
            seq: AtomicU16::new(1),
        }
    }

    /// Produce the next SSRC as a `u32` and its 10-digit string form.
    pub fn next(&self, kind: SsrcKind) -> (u32, String) {
        let head = match kind {
            SsrcKind::Live => '0',
            SsrcKind::Playback => '1',
        };
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) % 10000;
        let s = format!("{head}{}{:04}", self.domain5, seq);
        let n: u32 = s.parse().unwrap_or(0);
        (n, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_20_digit_code() {
        let parts = GbCode::new("34020000001320000001").parse().unwrap();
        assert_eq!(parts.region, "34020000");
        assert_eq!(parts.type_code, "132");
        assert_eq!(parts.sequence, "0000001");
    }

    #[test]
    fn opaque_code_never_lost_even_if_unparseable() {
        let c = GbCode::new("not-a-code");
        assert!(c.parse().is_none());
        assert_eq!(c.as_str(), "not-a-code"); // still usable
    }

    #[test]
    fn ssrc_is_10_digits_with_kind_prefix_and_domain() {
        let g = SsrcGenerator::new("34020000002000000001");
        let (n, s) = g.next(SsrcKind::Live);
        assert_eq!(s.len(), 10);
        assert!(s.starts_with('0')); // live
        assert_eq!(&s[1..6], "20000"); // domain digits [3..8]
        assert_eq!(s, "0200000001");
        assert_eq!(n, 200000001);
        let (_, s2) = g.next(SsrcKind::Playback);
        assert!(s2.starts_with('1')); // playback
        assert_eq!(&s2[6..], "0002"); // sequence advanced
    }
}
