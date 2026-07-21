use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use sha1::{Digest, Sha1};

/// The fields extracted from a WS-Security UsernameToken (PasswordDigest mode).
#[derive(Clone, Debug, PartialEq)]
pub struct UsernameToken {
    pub username: String,
    pub password_digest: String,
    pub nonce: String,   // base64 as it appears in the XML
    pub created: String, // the UTC timestamp string, verbatim
}

/// PasswordDigest = Base64( SHA1( decode(nonce) ++ created ++ password ) ).
pub fn password_digest(nonce_b64: &str, created: &str, password: &str) -> String {
    let nonce = B64.decode(nonce_b64).unwrap_or_default();
    let mut h = Sha1::new();
    h.update(&nonce);
    h.update(created.as_bytes());
    h.update(password.as_bytes());
    B64.encode(h.finalize())
}

/// True iff the token's username matches and its digest recomputes correctly.
pub fn verify(token: &UsernameToken, cfg_user: &str, cfg_pass: &str) -> bool {
    token.username == cfg_user
        && token.password_digest == password_digest(&token.nonce, &token.created, cfg_pass)
}

#[cfg(test)]
#[path = "auth_test.rs"]
mod auth_test;
