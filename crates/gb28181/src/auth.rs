//! SIP digest (RFC 2617, MD5) computation + auth policy.

use md5::{Digest, Md5};

/// Synchronous password-lookup callback: takes a device id, returns the password if known.
pub type PasswordProvider = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// How the server decides credentials (spec §3 row 6). The password provider is
/// synchronous: the consumer looks it up from its own store.
pub enum AuthConfig {
    Open,
    Shared(String),
    Provider(PasswordProvider),
}

impl AuthConfig {
    /// Resolve the expected password for a device id, or None if auth is Open.
    pub fn password_for(&self, device_id: &str) -> AuthDecision {
        match self {
            AuthConfig::Open => AuthDecision::Allow,
            AuthConfig::Shared(pw) => AuthDecision::Require(pw.clone()),
            AuthConfig::Provider(f) => match f(device_id) {
                Some(pw) => AuthDecision::Require(pw),
                None => AuthDecision::Reject,
            },
        }
    }
}

pub enum AuthDecision {
    Allow,           // no auth required
    Require(String), // verify against this password
    Reject,          // no credential known -> 403/401 with no valid answer
}

fn md5_hex(input: &str) -> String {
    let mut h = Md5::new();
    h.update(input.as_bytes());
    let out = h.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compute the RFC2617 "response" for qop-less digest:
/// HA1 = MD5(username:realm:password); HA2 = MD5(method:uri);
/// response = MD5(HA1:nonce:HA2).
pub fn digest_response(
    username: &str,
    realm: &str,
    password: &str,
    method: &str,
    uri: &str,
    nonce: &str,
) -> String {
    let ha1 = md5_hex(&format!("{username}:{realm}:{password}"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    md5_hex(&format!("{ha1}:{nonce}:{ha2}"))
}

/// Verify a client-supplied response against the expected password.
pub fn verify(
    username: &str,
    realm: &str,
    password: &str,
    method: &str,
    uri: &str,
    nonce: &str,
    client_response: &str,
) -> bool {
    digest_response(username, realm, password, method, uri, nonce)
        .eq_ignore_ascii_case(client_response)
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 2617 §3.5 canonical example vector.
    #[test]
    fn rfc2617_known_vector() {
        let resp = digest_response(
            "Mufasa",
            "testrealm@host.com",
            "Circle Of Life",
            "GET",
            "/dir/index.html",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",
        );
        // qop-less HA2 = MD5("GET:/dir/index.html"); this is the classic value.
        assert_eq!(resp, "670fd8c2df070c60b045671b8b24ff02");
    }

    #[test]
    fn verify_is_case_insensitive() {
        let n = "abc123";
        let good = digest_response(
            "34020000001320000001",
            "3402000000",
            "pw",
            "REGISTER",
            "sip:3402000000",
            n,
        );
        assert!(verify(
            "34020000001320000001",
            "3402000000",
            "pw",
            "REGISTER",
            "sip:3402000000",
            n,
            &good.to_uppercase()
        ));
    }

    #[test]
    fn config_open_allows() {
        let cfg = AuthConfig::Open;
        assert!(matches!(cfg.password_for("any"), AuthDecision::Allow));
    }

    #[test]
    fn config_provider_lookup() {
        let cfg =
            AuthConfig::Provider(Box::new(|id| (id == "known").then(|| "secret".to_string())));
        assert!(matches!(cfg.password_for("known"), AuthDecision::Require(p) if p == "secret"));
        assert!(matches!(cfg.password_for("unknown"), AuthDecision::Reject));
    }
}
