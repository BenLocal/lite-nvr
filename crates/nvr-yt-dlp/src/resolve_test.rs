use std::path::PathBuf;
use std::time::Duration;

use super::*;

/// Serializes the spawning tests: writing a fake script while another test's
/// fork/exec is in flight makes the exec fail with ETXTBSY (the forked child
/// briefly inherits the script's write fd).
static SPAWN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Writes an executable `#!/bin/sh` script standing in for yt-dlp.
fn fake_bin(tag: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!("nvr-yt-dlp-fake-{}-{tag}", std::process::id()));
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

const SAMPLE_INFO: &str = r#"{"url":"https://pull-cdn.example.com/live/room123.flv?sign=abc&expire=1789","http_headers":{"Referer":"https://live.douyin.com/","User-Agent":"Mozilla/5.0"},"is_live":true,"title":"room title","protocol":"https"}"#;

#[test]
fn resolve_args_defaults() {
    let args = YtDlp::with_bin("yt-dlp").resolve_args("https://live.douyin.com/123");
    let args: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
    assert_eq!(
        args,
        vec![
            "-j",
            "--no-warnings",
            "--no-playlist",
            "-f",
            "b",
            "--",
            "https://live.douyin.com/123",
        ]
    );
}

#[test]
fn resolve_args_with_options() {
    let args = YtDlp::with_bin("yt-dlp")
        .format("best[height<=720]")
        .cookies("/data/cookies.txt")
        .extra_arg("--proxy")
        .extra_arg("socks5://127.0.0.1:1080")
        .resolve_args("https://live.douyin.com/123");
    let args: Vec<&str> = args.iter().map(|a| a.to_str().unwrap()).collect();
    assert_eq!(
        args,
        vec![
            "-j",
            "--no-warnings",
            "--no-playlist",
            "-f",
            "best[height<=720]",
            "--cookies",
            "/data/cookies.txt",
            "--proxy",
            "socks5://127.0.0.1:1080",
            "--",
            "https://live.douyin.com/123",
        ]
    );
}

#[test]
fn parse_info_full() {
    let info = parse_info(SAMPLE_INFO).unwrap();
    assert_eq!(
        info.url,
        "https://pull-cdn.example.com/live/room123.flv?sign=abc&expire=1789"
    );
    assert_eq!(
        info.http_headers.get("Referer").map(String::as_str),
        Some("https://live.douyin.com/")
    );
    assert!(info.is_live);
    assert_eq!(info.title.as_deref(), Some("room title"));
    assert_eq!(info.protocol.as_deref(), Some("https"));
}

#[test]
fn parse_info_minimal() {
    let info = parse_info(r#"{"url":"https://cdn/x.m3u8"}"#).unwrap();
    assert_eq!(info.url, "https://cdn/x.m3u8");
    assert!(info.http_headers.is_empty());
    assert!(!info.is_live);
    assert_eq!(info.title, None);
}

#[test]
fn parse_info_empty_output() {
    let err = parse_info("\n  \n").unwrap_err();
    assert!(matches!(err, YtDlpError::Parse(_)), "{err}");
}

#[test]
fn parse_info_split_av_formats() {
    let err = parse_info(r#"{"requested_formats":[{},{}],"title":"t"}"#).unwrap_err();
    let YtDlpError::Parse(msg) = err else {
        panic!("expected Parse error");
    };
    assert!(msg.contains("muxed"), "{msg}");
}

#[tokio::test]
async fn resolve_via_fake_bin() {
    let _guard = SPAWN_LOCK.lock().await;
    let bin = fake_bin("ok", &format!("echo '{SAMPLE_INFO}'"));
    let info = YtDlp::with_bin(&bin)
        .resolve("https://live.douyin.com/123")
        .await
        .unwrap();
    assert!(info.url.starts_with("https://pull-cdn.example.com/"));
    assert!(info.is_live);
    std::fs::remove_file(bin).ok();
}

#[tokio::test]
async fn resolve_reports_stderr_on_failure() {
    let _guard = SPAWN_LOCK.lock().await;
    let bin = fake_bin("fail", "echo 'ERROR: Unsupported URL' >&2; exit 1");
    let err = YtDlp::with_bin(&bin)
        .resolve("https://example.com/nope")
        .await
        .unwrap_err();
    let YtDlpError::Failed { stderr, .. } = err else {
        panic!("expected Failed, got {err}");
    };
    assert!(stderr.contains("Unsupported URL"), "{stderr}");
    std::fs::remove_file(bin).ok();
}

#[tokio::test]
async fn resolve_missing_binary() {
    let _guard = SPAWN_LOCK.lock().await;
    let err = YtDlp::with_bin("/nonexistent/yt-dlp")
        .resolve("https://example.com")
        .await
        .unwrap_err();
    assert!(matches!(err, YtDlpError::Spawn { .. }), "{err}");
}

#[tokio::test]
async fn resolve_times_out() {
    let _guard = SPAWN_LOCK.lock().await;
    let bin = fake_bin("slow", "sleep 5");
    let err = YtDlp::with_bin(&bin)
        .timeout(Duration::from_millis(200))
        .resolve("https://example.com")
        .await
        .unwrap_err();
    assert!(matches!(err, YtDlpError::Timeout(_)), "{err}");
    std::fs::remove_file(bin).ok();
}

#[tokio::test]
async fn version_via_fake_bin() {
    let _guard = SPAWN_LOCK.lock().await;
    let bin = fake_bin("version", "echo '2026.07.01'");
    let version = YtDlp::with_bin(&bin).version().await.unwrap();
    assert_eq!(version, "2026.07.01");
    std::fs::remove_file(bin).ok();
}
