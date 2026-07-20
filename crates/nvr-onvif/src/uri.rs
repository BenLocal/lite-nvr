/// Inject `username:password@` into an RTSP URI's authority. ONVIF
/// `GetStreamUri` returns a credential-less URI, but many cameras still require
/// RTSP auth with the same credentials, so ffmpeg needs them in the URL.
///
/// Leaves unchanged: a URI that already has `@` in its authority, a non-rtsp
/// scheme, or empty credentials. Percent-encodes the user/pass.
pub fn inject_credentials(uri: &str, username: &str, password: &str) -> String {
    if username.is_empty() || !uri.starts_with("rtsp://") {
        return uri.to_string();
    }
    let rest = &uri["rtsp://".len()..];
    // Already has userinfo (authority contains '@' before the first '/').
    let authority_end = rest.find('/').unwrap_or(rest.len());
    if rest[..authority_end].contains('@') {
        return uri.to_string();
    }
    format!("rtsp://{}:{}@{}", encode(username), encode(password), rest)
}

/// Minimal RFC3986 userinfo percent-encoding: keep unreserved chars, encode the
/// rest (notably `:` `@` `/` `?` `#` and anything non-ASCII-alphanumeric).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~');
        if unreserved {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

#[cfg(test)]
#[path = "uri_test.rs"]
mod uri_test;
