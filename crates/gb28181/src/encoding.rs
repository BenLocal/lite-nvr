//! Decode a GB XML body to a UTF-8 String, honoring the declared charset.

use crate::error::{GbError, Result};

/// Decode raw XML bytes to UTF-8, honoring an `encoding="..."` declaration
/// (GB2312/GBK/UTF-8). Defaults to UTF-8 when no declaration is present.
pub fn decode_xml(bytes: &[u8]) -> Result<String> {
    let label = sniff_encoding(bytes).unwrap_or("utf-8");
    let enc = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    let (cow, _, had_errors) = enc.decode(bytes);
    if had_errors {
        return Err(GbError::XmlDecode(format!("charset {label} decode error")));
    }
    Ok(cow.into_owned())
}

/// Extract the encoding label from the XML prolog, if any. Only scans the first
/// ~128 bytes and does a case-insensitive search for `encoding="..."`.
fn sniff_encoding(bytes: &[u8]) -> Option<&'static str> {
    let head = &bytes[..bytes.len().min(128)];
    let head = String::from_utf8_lossy(head).to_ascii_lowercase();
    let idx = head.find("encoding=")?;
    let rest = &head[idx + "encoding=".len()..];
    let quote = rest.chars().next()?; // ' or "
    let val: String = rest[quote.len_utf8()..]
        .chars()
        .take_while(|&c| c != quote)
        .collect();
    match val.as_str() {
        "gb18030" => Some("gb18030"),
        "gb2312" | "gbk" => Some("gbk"),
        "utf-8" | "utf8" => Some("utf-8"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plain_utf8() {
        let xml = br#"<?xml version="1.0"?><Root><Name>cam</Name></Root>"#;
        let s = decode_xml(xml).unwrap();
        assert!(s.contains("<Name>cam</Name>"));
    }

    #[test]
    fn decodes_gb2312_declared_body() {
        // "摄像机" encoded in GBK
        let (gbk_bytes, _, _) = encoding_rs::GBK.encode("摄像机");
        let mut xml = br#"<?xml version="1.0" encoding="GB2312"?><Name>"#.to_vec();
        xml.extend_from_slice(&gbk_bytes);
        xml.extend_from_slice(b"</Name>");
        let s = decode_xml(&xml).unwrap();
        assert!(s.contains("摄像机"), "got: {s}");
    }

    #[test]
    fn missing_declaration_defaults_utf8() {
        let s = decode_xml("<A>x</A>".as_bytes()).unwrap();
        assert_eq!(s, "<A>x</A>");
    }

    #[test]
    fn invalid_utf8_in_encoding_attribute_does_not_panic() {
        // \xff\xfe immediately after `encoding=` is invalid UTF-8; from_utf8_lossy
        // replaces it with U+FFFD (3 bytes), and the old `rest[1..]` would slice
        // into the middle of that codepoint.  The fix uses `quote.len_utf8()`.
        let body: &[u8] = b"<?xml version=\"1.0\" encoding=\xff\xfe?><A>x</A>";
        // Must not panic — Ok or Err are both acceptable.
        let _result = decode_xml(body);
    }
}
