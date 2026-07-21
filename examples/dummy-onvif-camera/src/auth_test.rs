use super::*;

fn token(user: &str, nonce: &str, created: &str, pass: &str) -> UsernameToken {
    UsernameToken {
        username: user.to_string(),
        password_digest: password_digest(nonce, created, pass),
        nonce: nonce.to_string(),
        created: created.to_string(),
    }
}

#[test]
fn accepts_correct_credentials() {
    // nonce is base64 of some bytes; created is any string.
    let t = token(
        "admin",
        "MTIzNDU2Nzg5MDEyMzQ1Ng==",
        "2026-07-20T00:00:00Z",
        "secret",
    );
    assert!(verify(&t, "admin", "secret"));
}

#[test]
fn rejects_wrong_password() {
    let t = token(
        "admin",
        "MTIzNDU2Nzg5MDEyMzQ1Ng==",
        "2026-07-20T00:00:00Z",
        "secret",
    );
    assert!(!verify(&t, "admin", "WRONG"));
}

#[test]
fn rejects_wrong_username() {
    let t = token(
        "admin",
        "MTIzNDU2Nzg5MDEyMzQ1Ng==",
        "2026-07-20T00:00:00Z",
        "secret",
    );
    assert!(!verify(&t, "root", "secret"));
}

#[test]
fn digest_is_deterministic_and_known() {
    // SHA1("" nonce-bytes are empty for empty b64 || "C" || "P") sanity: same inputs -> same digest.
    let a = password_digest("", "C", "P");
    let b = password_digest("", "C", "P");
    assert_eq!(a, b);
    assert!(!a.is_empty());
}
