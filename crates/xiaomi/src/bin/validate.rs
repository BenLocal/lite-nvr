//! Validation CLI for the `xiaomi` crate phases 1-3 (no camera needed).
//!
//! Runs cloud login (handling captcha / 2-step verification interactively),
//! lists the account's cameras, and resolves each via `miss_get_vendor` — so you
//! can confirm auth + device resolution work against the real Xiaomi servers and
//! capture each camera's `did` / `model` / `vendor` / `ip` for phases 4-6.
//!
//! Usage:
//!   XIAOMI_USER=<id> XIAOMI_PASS=<pw> [XIAOMI_REGION=us] \
//!     cargo run -p xiaomi --bin validate
//!   (or pass user/pass as the first two args; region defaults to "" = CN)
//!
//! Note: the password is read in plain text (no masking) — this is a local
//! validation tool.

use std::io::{self, Write};

use anyhow::Result;
use xiaomi::cloud::{Cloud, LoginStep};
use xiaomi::device::{self, APP_XIAOMI_HOME};

fn prompt(msg: &str) -> Result<String> {
    print!("{msg}");
    io::stdout().flush()?;
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let user = env_or(std::env::var("XIAOMI_USER").ok(), args.next());
    let pass = env_or(std::env::var("XIAOMI_PASS").ok(), args.next());
    let region = std::env::var("XIAOMI_REGION").unwrap_or_default();

    let user = non_empty_or_prompt(user, "Xiaomi username (email/phone/account id): ")?;
    let pass = non_empty_or_prompt(pass, "Xiaomi password: ")?;

    let mut cloud = Cloud::new(APP_XIAOMI_HOME)?;
    println!("Logging in (region={:?})...", region);
    let mut step = cloud.login(&user, &pass)?;
    loop {
        match step {
            LoginStep::Done => break,
            LoginStep::Captcha(bytes) => {
                let path = std::env::temp_dir().join("xiaomi_captcha.png");
                std::fs::write(&path, &bytes)?;
                println!(
                    "Captcha image saved to {} — open it and enter the code.",
                    path.display()
                );
                let code = prompt("captcha code: ")?;
                step = cloud.login_with_captcha(&code)?;
            }
            LoginStep::Verify {
                masked_phone,
                masked_email,
            } => {
                println!(
                    "Two-step verification — a ticket was sent to {}{}",
                    masked_phone, masked_email
                );
                let ticket = prompt("verification ticket: ")?;
                step = cloud.login_with_verify(&ticket)?;
            }
        }
    }

    println!("\n✓ login OK");
    let (uid, token) = cloud.user_token();
    println!("  user_id = {uid}");
    println!("  token   = {token}");
    println!("  region  = {region:?}");
    println!("  (reuse without re-login via Cloud::login_with_token(user_id, token))");

    println!("\nListing cameras...");
    let cams = device::list_cameras(&cloud, &region)?;
    if cams.is_empty() {
        println!("  (no camera devices found on this account)");
    }
    for cam in &cams {
        println!(
            "\n- {} | did={} model={} ip={}",
            if cam.name.is_empty() {
                "<unnamed>"
            } else {
                &cam.name
            },
            cam.did,
            cam.model,
            cam.ip
        );
        match device::resolve_miss(&cloud, &region, &cam.did, &cam.model) {
            Ok(conn) => println!(
                "    resolved: vendor={} uid={:?} device_public={}B sign={}B",
                conn.vendor,
                conn.uid,
                conn.device_public.len(),
                conn.sign.len()
            ),
            Err(e) => println!("    resolve_miss FAILED: {e:#}"),
        }
    }

    println!("\nDone. Use the did/model/ip/vendor above for phases 4-6 (xiaomi -> ZLM).");
    Ok(())
}

fn env_or(env: Option<String>, arg: Option<String>) -> String {
    env.filter(|s| !s.is_empty()).or(arg).unwrap_or_default()
}

fn non_empty_or_prompt(value: String, msg: &str) -> Result<String> {
    if value.is_empty() {
        prompt(msg)
    } else {
        Ok(value)
    }
}
