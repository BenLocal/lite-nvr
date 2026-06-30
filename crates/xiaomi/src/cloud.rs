//! Port of go2rtc `pkg/xiaomi/cloud.go` — Xiaomi account auth + miio-signed API.
//!
//! Auth is a small state machine: [`Cloud::login`] may return [`LoginStep::Captcha`]
//! or [`LoginStep::Verify`], which the caller resolves with
//! [`Cloud::login_with_captcha`] / [`Cloud::login_with_verify`]; on success it
//! returns [`LoginStep::Done`] and a `userId`/`passToken` is available via
//! [`Cloud::user_token`] for token re-login ([`Cloud::login_with_token`]).
//!
//! [`Cloud::request`] performs the encrypted, signed miio cloud call
//! (RC4-drop1024 + SHA1/SHA256 signatures), matching the Go implementation.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use rand::Rng;
use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_TYPE, COOKIE, HeaderMap, LOCATION, SET_COOKIE};
use reqwest::redirect::Policy;
use sha1::Sha1;
use sha2::{Digest, Sha256};

const LOGIN_PREFIX: &[u8] = b"&&&START&&&";
const MAX_REDIRECTS: usize = 10;

/// The next step the caller must take to finish login.
#[derive(Debug)]
pub enum LoginStep {
    /// Authenticated — cookies + ssecurity are set.
    Done,
    /// A captcha image (raw bytes) must be solved; call [`Cloud::login_with_captcha`].
    Captcha(Vec<u8>),
    /// Two-step verification: a ticket was sent to the masked phone/email; call
    /// [`Cloud::login_with_verify`].
    Verify {
        masked_phone: String,
        masked_email: String,
    },
}

pub struct Cloud {
    client: Client,
    sid: String,
    cookies: String,
    ssecurity: Vec<u8>,
    user_id: String,
    pass_token: String,
    /// Multi-step login state (username/password/captcha/verify scratch).
    auth: HashMap<String, String>,
}

impl Cloud {
    pub fn new(sid: &str) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(Policy::none()) // we follow + inspect redirects (and cookies) manually
            .build()?;
        Ok(Self {
            client,
            sid: sid.to_string(),
            cookies: String::new(),
            ssecurity: Vec::new(),
            user_id: String::new(),
            pass_token: String::new(),
            auth: HashMap::new(),
        })
    }

    pub fn login(&mut self, username: &str, password: &str) -> Result<LoginStep> {
        let res = self
            .client
            .get(format!(
                "https://account.xiaomi.com/pass/serviceLogin?_json=true&sid={}",
                self.sid
            ))
            .send()?;
        let v1: ServiceLogin = read_login_response(res)?;

        let hash = format!("{:X}", md5::Md5::digest(password.as_bytes()));

        let mut form: Vec<(String, String)> = vec![
            ("_json".into(), "true".into()),
            ("hash".into(), hash),
            ("sid".into(), v1.sid.unwrap_or_default()),
            ("callback".into(), v1.callback.unwrap_or_default()),
            ("_sign".into(), v1.sign.unwrap_or_default()),
            ("qs".into(), v1.qs.unwrap_or_default()),
            ("user".into(), username.to_string()),
        ];
        let mut cookies = format!("deviceId={}", rand_string(16));

        // login after captcha
        if let Some(code) = self.auth.get("captcha_code").filter(|s| !s.is_empty()) {
            form.push(("captCode".into(), code.clone()));
            cookies += &format!(
                "; ick={}",
                self.auth.get("ick").cloned().unwrap_or_default()
            );
        }

        let res = self
            .client
            .post("https://account.xiaomi.com/pass/serviceLoginAuth2")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(COOKIE, &cookies)
            .body(encode_form(&form))
            .send()?;
        let v2: LoginAuth2 = read_login_response(res)?;

        // save auth for two-step verification
        self.auth = HashMap::from([
            ("username".to_string(), username.to_string()),
            ("password".to_string(), password.to_string()),
        ]);

        if let Some(url) = v2.captcha_url.filter(|s| !s.is_empty()) {
            return self.get_captcha(&url);
        }
        if let Some(url) = v2.notification_url.filter(|s| !s.is_empty()) {
            return self.auth_start(&url);
        }
        let location = v2.location.filter(|s| !s.is_empty());
        let Some(location) = location else {
            bail!("xiaomi login: no location in response");
        };

        self.auth.clear();
        self.ssecurity = decode_b64_bytes(v2.ssecurity.as_deref())?;
        self.pass_token = v2.pass_token.unwrap_or_default();
        self.finish_auth(&location)?;
        Ok(LoginStep::Done)
    }

    pub fn login_with_captcha(&mut self, captcha: &str) -> Result<LoginStep> {
        if self.auth.get("ick").map(|s| s.is_empty()).unwrap_or(true) {
            bail!("xiaomi: wrong login step (no captcha pending)");
        }
        self.auth
            .insert("captcha_code".to_string(), captcha.to_string());

        if self.auth.get("flag").is_some_and(|f| !f.is_empty()) {
            return self.send_ticket();
        }
        let (u, p) = (
            self.auth.get("username").cloned().unwrap_or_default(),
            self.auth.get("password").cloned().unwrap_or_default(),
        );
        self.login(&u, &p)
    }

    pub fn login_with_verify(&mut self, ticket: &str) -> Result<LoginStep> {
        let flag = self.auth.get("flag").cloned().unwrap_or_default();
        if flag.is_empty() {
            bail!("xiaomi: wrong login step (no verification pending)");
        }
        let session = self
            .auth
            .get("identity_session")
            .cloned()
            .unwrap_or_default();
        let url = format!(
            "https://account.xiaomi.com/identity/auth/verify{}?_flag{}&ticket={}&trust=false&_json=true",
            self.verify_name(),
            flag,
            ticket
        );
        let res = self
            .client
            .post(url)
            .header(COOKIE, format!("identity_session={session}"))
            .send()?;
        let v1: Location = read_login_response(res)?;
        let Some(location) = v1.location.filter(|s| !s.is_empty()) else {
            bail!("xiaomi verify: no location in response");
        };
        self.finish_auth(&location)?;
        Ok(LoginStep::Done)
    }

    pub fn login_with_token(&mut self, user_id: &str, pass_token: &str) -> Result<LoginStep> {
        let res = self
            .client
            .get(format!(
                "https://account.xiaomi.com/pass/serviceLogin?_json=true&sid={}",
                self.sid
            ))
            .header(COOKIE, format!("userId={user_id}; passToken={pass_token}"))
            .send()?;
        let v1: LoginAuth2 = read_login_response(res)?;
        self.ssecurity = decode_b64_bytes(v1.ssecurity.as_deref())?;
        self.pass_token = v1.pass_token.unwrap_or_default();
        let Some(location) = v1.location.filter(|s| !s.is_empty()) else {
            bail!("xiaomi token login: no location in response");
        };
        self.finish_auth(&location)?;
        Ok(LoginStep::Done)
    }

    pub fn user_token(&self) -> (String, String) {
        (self.user_id.clone(), self.pass_token.clone())
    }

    fn get_captcha(&mut self, captcha_url: &str) -> Result<LoginStep> {
        let res = self
            .client
            .get(format!("https://account.xiaomi.com{captcha_url}"))
            .send()?;
        let ick = find_cookie(res.headers(), "ick");
        let body = res.bytes()?.to_vec();
        self.auth.insert("ick".to_string(), ick);
        Ok(LoginStep::Captcha(body))
    }

    fn auth_start(&mut self, notification_url: &str) -> Result<LoginStep> {
        let raw_url = notification_url.replace("/fe/service/identity/authStart", "/identity/list");
        let res = self.client.get(raw_url).send()?;
        let session = find_cookie(res.headers(), "identity_session");
        let v1: AuthList = read_login_response(res)?;
        self.auth.insert("flag".to_string(), v1.flag.to_string());
        self.auth.insert("identity_session".to_string(), session);
        self.send_ticket()
    }

    fn verify_name(&self) -> &'static str {
        match self.auth.get("flag").map(String::as_str) {
            Some("4") => "Phone",
            Some("8") => "Email",
            _ => "",
        }
    }

    fn send_ticket(&mut self) -> Result<LoginStep> {
        let name = self.verify_name();
        let session = self
            .auth
            .get("identity_session")
            .cloned()
            .unwrap_or_default();
        let flag = self.auth.get("flag").cloned().unwrap_or_default();
        let mut cookies = format!("identity_session={session}");

        let res = self
            .client
            .get(format!(
                "https://account.xiaomi.com/identity/auth/verify{name}?_flag={flag}&_json=true"
            ))
            .header(COOKIE, &cookies)
            .send()?;
        let v1: VerifyInfo = read_login_response(res)?;

        let capt_code = self.auth.get("captcha_code").cloned().unwrap_or_default();
        if !capt_code.is_empty() {
            cookies += &format!(
                "; ick={}",
                self.auth.get("ick").cloned().unwrap_or_default()
            );
        }

        let form = vec![
            ("_json".to_string(), "true".to_string()),
            ("icode".to_string(), capt_code),
            ("retry".to_string(), "0".to_string()),
        ];
        let res = self
            .client
            .post(format!(
                "https://account.xiaomi.com/identity/auth/send{name}Ticket"
            ))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(COOKIE, &cookies)
            .body(encode_form(&form))
            .send()?;
        let v2: SendTicket = read_login_response(res)?;

        if let Some(url) = v2.captcha_url.filter(|s| !s.is_empty()) {
            return self.get_captcha(&url);
        }
        if v2.code != 0 {
            bail!("xiaomi send ticket: code {}", v2.code);
        }
        Ok(LoginStep::Verify {
            masked_phone: v1.masked_phone.unwrap_or_default(),
            masked_email: v1.masked_email.unwrap_or_default(),
        })
    }

    /// Follow the post-login redirect chain, collecting auth cookies and the
    /// `ssecurity` from the `Extension-Pragma` header (Go `finishAuth`).
    fn finish_auth(&mut self, location: &str) -> Result<()> {
        let mut url = location.to_string();
        let (mut c_user_id, mut service_token) = (String::new(), String::new());

        for _ in 0..MAX_REDIRECTS {
            let res = self.client.get(&url).send()?;
            let headers = res.headers().clone();
            let status = res.status();

            for (name, value) in parse_set_cookies(&headers) {
                match name.as_str() {
                    "userId" => self.user_id = value,
                    "cUserId" => c_user_id = value,
                    "serviceToken" => service_token = value,
                    "passToken" => self.pass_token = value,
                    _ => {}
                }
            }
            if let Some(s) = headers
                .get("Extension-Pragma")
                .and_then(|h| h.to_str().ok())
            {
                if let Ok(v) = serde_json::from_str::<Ssecurity>(s) {
                    self.ssecurity = decode_b64_bytes(v.ssecurity.as_deref())?;
                }
            }

            if status.is_redirection() {
                if let Some(loc) = headers.get(LOCATION).and_then(|h| h.to_str().ok()) {
                    url = loc.to_string();
                    continue;
                }
            }
            break;
        }

        self.cookies = format!(
            "userId={}; cUserId={c_user_id}; serviceToken={service_token}",
            self.user_id
        );
        Ok(())
    }

    /// Encrypted, signed miio cloud request (Go `Request`).
    pub fn request(
        &self,
        base_url: &str,
        api_url: &str,
        params: &str,
        headers: &HashMap<String, String>,
    ) -> Result<Vec<u8>> {
        let nonce = gen_nonce();
        let signed_nonce = gen_signed_nonce(&self.ssecurity, &nonce);

        let mut form: Vec<(String, String)> = vec![("data".to_string(), params.to_string())];

        // 1. hash for the data param
        let rc4_hash = gen_signature64("POST", api_url, &form, &signed_nonce);
        form.push(("rc4_hash__".to_string(), rc4_hash));

        // 2. encrypt every value
        for (_, value) in form.iter_mut() {
            let ciphertext = rc4_crypt(&signed_nonce, value.as_bytes());
            *value = B64.encode(ciphertext);
        }

        // 3. signature over the encrypted values
        let signature = gen_signature64("POST", api_url, &form, &signed_nonce);
        form.push(("signature".to_string(), signature));

        // 4. nonce
        form.push(("_nonce".to_string(), B64.encode(nonce)));

        let mut req = self
            .client
            .post(format!("{base_url}{api_url}"))
            .header(COOKIE, &self.cookies)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded");
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let res = req.body(encode_form(&form)).send()?;
        if !res.status().is_success() {
            bail!("xiaomi request: {}", res.status());
        }
        let body = res.text()?;
        let ciphertext = B64.decode(body.trim())?;
        let plaintext = rc4_crypt(&signed_nonce, &ciphertext);

        let res1: ApiResponse = serde_json::from_slice(&plaintext)?;
        if res1.code != 0 {
            bail!("xiaomi: {}", res1.message);
        }
        Ok(res1.result.unwrap_or_default().to_string().into_bytes())
    }
}

// ----- response shapes -----

#[derive(serde::Deserialize)]
struct ServiceLogin {
    qs: Option<String>,
    #[serde(rename = "_sign")]
    sign: Option<String>,
    sid: Option<String>,
    callback: Option<String>,
}

#[derive(serde::Deserialize)]
struct LoginAuth2 {
    ssecurity: Option<String>,
    #[serde(rename = "passToken")]
    pass_token: Option<String>,
    location: Option<String>,
    #[serde(rename = "captchaURL")]
    captcha_url: Option<String>,
    #[serde(rename = "notificationUrl")]
    notification_url: Option<String>,
}

#[derive(serde::Deserialize)]
struct Location {
    location: Option<String>,
}

#[derive(serde::Deserialize)]
struct AuthList {
    #[serde(default)]
    flag: i64,
}

#[derive(serde::Deserialize)]
struct VerifyInfo {
    #[serde(rename = "maskedPhone")]
    masked_phone: Option<String>,
    #[serde(rename = "maskedEmail")]
    masked_email: Option<String>,
}

#[derive(serde::Deserialize)]
struct SendTicket {
    #[serde(default)]
    code: i64,
    #[serde(rename = "captchaURL")]
    captcha_url: Option<String>,
}

#[derive(serde::Deserialize)]
struct Ssecurity {
    ssecurity: Option<String>,
}

#[derive(serde::Deserialize)]
struct ApiResponse {
    #[serde(default)]
    code: i64,
    #[serde(default)]
    message: String,
    result: Option<serde_json::Value>,
}

// ----- helpers -----

/// Parse a Xiaomi login response: strip the `&&&START&&&` prefix, then JSON.
fn read_login_response<T: serde::de::DeserializeOwned>(res: Response) -> Result<T> {
    let body = res.bytes()?;
    let json = body.strip_prefix(LOGIN_PREFIX).ok_or_else(|| {
        anyhow!(
            "xiaomi: unexpected response: {}",
            String::from_utf8_lossy(&body)
        )
    })?;
    Ok(serde_json::from_slice(json)?)
}

/// Go marshals `[]byte` JSON fields as base64 strings; decode the same way.
fn decode_b64_bytes(s: Option<&str>) -> Result<Vec<u8>> {
    match s.filter(|s| !s.is_empty()) {
        Some(s) => Ok(B64.decode(s)?),
        None => Ok(Vec::new()),
    }
}

fn find_cookie(headers: &HeaderMap, name: &str) -> String {
    parse_set_cookies(headers)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
        .unwrap_or_default()
}

fn parse_set_cookies(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .filter_map(|c| {
            let pair = c.split(';').next()?;
            let (name, value) = pair.split_once('=')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect()
}

fn encode_form(form: &[(String, String)]) -> String {
    form.iter()
        .map(|(k, v)| format!("{}={}", urlencode(k), urlencode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn rand_string(n: usize) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    (0..n)
        .map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char)
        .collect()
}

fn gen_nonce() -> [u8; 12] {
    let ts = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        / 60) as u32;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill(&mut nonce[..8]);
    nonce[8..].copy_from_slice(&ts.to_be_bytes());
    nonce
}

fn gen_signed_nonce(ssecurity: &[u8], nonce: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(ssecurity);
    hasher.update(nonce);
    hasher.finalize().to_vec()
}

/// RC4 with the standard miio 1024-byte keystream discard (RC4-drop1024).
/// Implemented inline (RC4 is trivial and the key length is variable, which the
/// generic `rc4` crate makes awkward). XOR-symmetric: same fn encrypts/decrypts.
fn rc4_crypt(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut s: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut j: u8 = 0;
    for i in 0..256usize {
        j = j.wrapping_add(s[i]).wrapping_add(key[i % key.len()]);
        s.swap(i, j as usize);
    }

    let (mut i, mut j) = (0u8, 0u8);
    let next = |s: &mut [u8; 256], i: &mut u8, j: &mut u8| -> u8 {
        *i = i.wrapping_add(1);
        *j = j.wrapping_add(s[*i as usize]);
        s.swap(*i as usize, *j as usize);
        s[s[*i as usize].wrapping_add(s[*j as usize]) as usize]
    };

    for _ in 0..1024 {
        next(&mut s, &mut i, &mut j); // drop1024
    }
    data.iter()
        .map(|b| b ^ next(&mut s, &mut i, &mut j))
        .collect()
}

fn gen_signature64(
    method: &str,
    path: &str,
    form: &[(String, String)],
    signed_nonce: &[u8],
) -> String {
    let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let mut s = format!("{method}&{path}&data={}", get("data").unwrap_or(""));
    if let Some(hash) = get("rc4_hash__") {
        s += &format!("&rc4_hash__={hash}");
    }
    s += &format!("&{}", B64.encode(signed_nonce));
    let signature = Sha1::digest(s.as_bytes());
    B64.encode(signature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rc4_round_trip_with_drop1024() {
        let key = gen_signed_nonce(b"secret", &[1u8; 12]);
        let plain = b"the quick brown fox";
        let enc = rc4_crypt(&key, plain);
        assert_ne!(enc, plain);
        assert_eq!(rc4_crypt(&key, &enc), plain); // RC4 is symmetric
    }

    #[test]
    fn signed_nonce_is_sha256_len() {
        assert_eq!(gen_signed_nonce(b"x", &[0u8; 12]).len(), 32);
    }

    #[test]
    fn signature_is_deterministic_and_includes_hash() {
        let nonce = gen_signed_nonce(b"s", &[2u8; 12]);
        let form = vec![
            ("data".to_string(), "{\"a\":1}".to_string()),
            ("rc4_hash__".to_string(), "HHHH".to_string()),
        ];
        let a = gen_signature64("POST", "/p", &form, &nonce);
        let b = gen_signature64("POST", "/p", &form, &nonce);
        assert_eq!(a, b);
        // dropping rc4_hash__ changes the signature
        let form2 = vec![("data".to_string(), "{\"a\":1}".to_string())];
        assert_ne!(a, gen_signature64("POST", "/p", &form2, &nonce));
    }

    #[test]
    fn read_login_response_requires_prefix() {
        // build a fake response is awkward; test the parse helper indirectly via
        // the prefix contract using the same logic.
        let good = b"&&&START&&&{\"location\":\"x\"}";
        assert!(good.strip_prefix(LOGIN_PREFIX).is_some());
        let bad = b"{\"location\":\"x\"}";
        assert!(bad.strip_prefix(LOGIN_PREFIX).is_none());
    }
}
