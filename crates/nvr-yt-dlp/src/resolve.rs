use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;

/// Environment variable overriding the `yt-dlp` binary location.
pub const YT_DLP_BIN_ENV: &str = "YT_DLP_BIN";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Best *muxed* format — a single URL carrying audio+video. Live streams
/// always expose one; separate audio/video formats would yield two URLs the
/// downstream single-input pipeline can't open.
const DEFAULT_FORMAT: &str = "b";

#[derive(Debug, thiserror::Error)]
pub enum YtDlpError {
    #[error("failed to spawn yt-dlp ({bin}): {source}")]
    Spawn {
        bin: String,
        #[source]
        source: std::io::Error,
    },
    #[error("yt-dlp did not finish within {0:?}")]
    Timeout(Duration),
    #[error("yt-dlp exited with {status}: {stderr}")]
    Failed {
        status: std::process::ExitStatus,
        stderr: String,
    },
    #[error("failed to parse yt-dlp output: {0}")]
    Parse(String),
    #[error("yt-dlp io error: {0}")]
    Io(#[from] std::io::Error),
}

/// A freshly resolved, immediately playable stream address.
///
/// The URL is typically temporary and signed — don't persist it; re-resolve
/// on every (re)connect.
#[derive(Debug, Clone)]
pub struct ResolvedStream {
    pub url: String,
    /// HTTP headers the CDN requires (Referer / User-Agent / Cookie …);
    /// pass them to the demuxer or the pull fails on picky CDNs.
    pub http_headers: HashMap<String, String>,
    pub is_live: bool,
    pub title: Option<String>,
    /// yt-dlp protocol tag, e.g. `https`, `m3u8_native`.
    pub protocol: Option<String>,
}

/// Configured handle to the external `yt-dlp` binary.
#[derive(Debug, Clone)]
pub struct YtDlp {
    bin: PathBuf,
    timeout: Duration,
    format: String,
    cookies: Option<PathBuf>,
    extra_args: Vec<String>,
}

impl Default for YtDlp {
    fn default() -> Self {
        Self::new()
    }
}

impl YtDlp {
    /// Uses the binary from `YT_DLP_BIN`, falling back to `yt-dlp` located on
    /// PATH via `which` (kept as the bare name if not found, so the failure
    /// surfaces as a clear [`YtDlpError::Spawn`] on first use).
    pub fn new() -> Self {
        let bin = std::env::var_os(YT_DLP_BIN_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| match which::which("yt-dlp") {
                Ok(path) => {
                    log::debug!("yt-dlp found at {}", path.display());
                    path
                }
                Err(e) => {
                    log::warn!("yt-dlp not found on PATH ({e}); keeping bare name");
                    PathBuf::from("yt-dlp")
                }
            });
        Self::with_bin(bin)
    }

    pub fn with_bin(bin: impl Into<PathBuf>) -> Self {
        Self {
            bin: bin.into(),
            timeout: DEFAULT_TIMEOUT,
            format: DEFAULT_FORMAT.to_string(),
            cookies: None,
            extra_args: Vec::new(),
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// yt-dlp format selector (`-f`). Must select a single muxed format.
    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.format = format.into();
        self
    }

    /// Netscape cookie file (`--cookies`) for rooms that need a login.
    pub fn cookies(mut self, file: impl Into<PathBuf>) -> Self {
        self.cookies = Some(file.into());
        self
    }

    /// Extra raw yt-dlp arguments appended before the URL.
    pub fn extra_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Resolves a room/page URL into the current playable stream address.
    pub async fn resolve(&self, page_url: &str) -> Result<ResolvedStream, YtDlpError> {
        let started = std::time::Instant::now();
        let stdout = self.run(self.resolve_args(page_url)).await?;
        let resolved = parse_info(&stdout)?;
        log::debug!(
            "yt-dlp resolved {page_url} in {:?} (live={}, protocol={:?})",
            started.elapsed(),
            resolved.is_live,
            resolved.protocol,
        );
        Ok(resolved)
    }

    /// Returns the yt-dlp version string; cheap availability probe.
    pub async fn version(&self) -> Result<String, YtDlpError> {
        let stdout = self.run(vec![OsString::from("--version")]).await?;
        Ok(stdout.trim().to_string())
    }

    fn resolve_args(&self, page_url: &str) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "-j".into(),
            "--no-warnings".into(),
            "--no-playlist".into(),
            "-f".into(),
            self.format.as_str().into(),
        ];
        if let Some(cookies) = &self.cookies {
            args.push("--cookies".into());
            args.push(cookies.into());
        }
        args.extend(self.extra_args.iter().map(OsString::from));
        args.push("--".into());
        args.push(page_url.into());
        args
    }

    async fn run(&self, args: Vec<OsString>) -> Result<String, YtDlpError> {
        let child = tokio::process::Command::new(&self.bin)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| YtDlpError::Spawn {
                bin: self.bin.display().to_string(),
                source,
            })?;
        let output = tokio::time::timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| YtDlpError::Timeout(self.timeout))??;
        if !output.status.success() {
            return Err(YtDlpError::Failed {
                status: output.status,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// The subset of yt-dlp's info dict (`-j` output) we care about.
#[derive(Debug, Deserialize)]
struct InfoDict {
    url: Option<String>,
    #[serde(default)]
    http_headers: HashMap<String, String>,
    #[serde(default)]
    is_live: bool,
    title: Option<String>,
    protocol: Option<String>,
    requested_formats: Option<serde_json::Value>,
}

fn parse_info(stdout: &str) -> Result<ResolvedStream, YtDlpError> {
    // `-j` prints one JSON object per line; --no-playlist keeps it to one.
    let line = stdout
        .lines()
        .find(|l| !l.trim().is_empty())
        .ok_or_else(|| YtDlpError::Parse("empty yt-dlp output".to_string()))?;
    let info: InfoDict =
        serde_json::from_str(line).map_err(|e| YtDlpError::Parse(e.to_string()))?;
    let Some(url) = info.url else {
        if info.requested_formats.is_some() {
            return Err(YtDlpError::Parse(
                "yt-dlp selected separate audio+video formats (no single muxed URL); \
                 use a muxed format selector like \"b\""
                    .to_string(),
            ));
        }
        return Err(YtDlpError::Parse(
            "no `url` field in yt-dlp output".to_string(),
        ));
    };
    Ok(ResolvedStream {
        url,
        http_headers: info.http_headers,
        is_live: info.is_live,
        title: info.title,
        protocol: info.protocol,
    })
}

#[cfg(test)]
#[path = "resolve_test.rs"]
mod resolve_test;
