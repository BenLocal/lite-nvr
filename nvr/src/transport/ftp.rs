//! FTP backend (blocking `suppaftp`, driven on the blocking pool).

use std::path::Path;

use anyhow::{Context, Result};
use suppaftp::FtpStream;

use crate::transport::backend::StorageBackend;
use crate::transport::config::FtpConfig;

pub struct FtpBackend {
    cfg: FtpConfig,
}

impl FtpBackend {
    pub fn from_json(json: &str) -> Result<Self> {
        Ok(Self {
            cfg: serde_json::from_str(json).context("parse ftp config")?,
        })
    }
}

fn connect(cfg: &FtpConfig) -> Result<FtpStream> {
    let mut ftp = FtpStream::connect((cfg.host.as_str(), cfg.port))
        .with_context(|| format!("connect ftp {}:{}", cfg.host, cfg.port))?;
    ftp.login(&cfg.username, &cfg.password)
        .context("ftp login")?;
    Ok(ftp)
}

/// Walk into (creating as needed) each parent directory of `remote_key`, then
/// return the leaf file name to `put_file` into the current directory.
fn enter_parent_dirs<'a>(ftp: &mut FtpStream, remote_key: &'a str) -> Result<&'a str> {
    let mut parts: Vec<&str> = remote_key.split('/').filter(|p| !p.is_empty()).collect();
    let file_name = parts.pop().context("empty remote key")?;
    let _ = ftp.cwd("/");
    for dir in parts {
        if ftp.cwd(dir).is_err() {
            ftp.mkdir(dir).ok();
            ftp.cwd(dir).with_context(|| format!("ftp cwd {dir}"))?;
        }
    }
    Ok(file_name)
}

fn upload_blocking(cfg: &FtpConfig, local: &Path, remote_key: &str) -> Result<()> {
    let mut ftp = connect(cfg)?;
    let file_name = enter_parent_dirs(&mut ftp, remote_key)?;
    let mut file =
        std::fs::File::open(local).with_context(|| format!("open {}", local.display()))?;
    ftp.put_file(file_name, &mut file).context("ftp put_file")?;
    let _ = ftp.quit();
    Ok(())
}

#[async_trait::async_trait]
impl StorageBackend for FtpBackend {
    async fn upload(&self, local_path: &Path, remote_key: &str) -> Result<()> {
        let cfg = self.cfg.clone();
        let local = local_path.to_path_buf();
        let remote = remote_key.to_string();
        tokio::task::spawn_blocking(move || upload_blocking(&cfg, &local, &remote))
            .await
            .context("ftp upload task")?
    }

    async fn test(&self) -> Result<()> {
        let cfg = self.cfg.clone();
        tokio::task::spawn_blocking(move || {
            let mut ftp = connect(&cfg)?;
            let _ = ftp.quit();
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("ftp test task")?
    }
}

#[cfg(test)]
#[path = "ftp_test.rs"]
mod ftp_test;
