//! SMB/CIFS backend (blocking `pavao` → libsmbclient, driven on the blocking
//! pool). The `SmbClient` holds a raw libsmbclient context and is not `Send`, so
//! it is created, used, and dropped entirely inside one `spawn_blocking` closure.

use std::io;
use std::path::Path;

use anyhow::{Context, Result};
use pavao::{SmbClient, SmbCredentials, SmbMode, SmbOpenOptions, SmbOptions};

use crate::transport::backend::StorageBackend;
use crate::transport::config::SmbConfig;

pub struct SmbBackend {
    cfg: SmbConfig,
}

impl SmbBackend {
    pub fn from_json(json: &str) -> Result<Self> {
        Ok(Self {
            cfg: serde_json::from_str(json).context("parse smb config")?,
        })
    }
}

fn client(cfg: &SmbConfig) -> Result<SmbClient> {
    let server = if cfg.host.starts_with("smb://") {
        cfg.host.clone()
    } else {
        format!("smb://{}", cfg.host)
    };
    SmbClient::new(
        SmbCredentials::default()
            .server(server)
            .share(format!("/{}", cfg.share.trim_start_matches('/')))
            .username(&cfg.username)
            .password(&cfg.password)
            .workgroup(&cfg.workgroup),
        SmbOptions::default().one_share_per_server(true),
    )
    .context("smb connect")
}

fn upload_blocking(cfg: &SmbConfig, local: &Path, remote_key: &str) -> Result<()> {
    let client = client(cfg)?;

    // Best-effort create of each parent directory (ignore "already exists").
    let parts: Vec<&str> = remote_key.split('/').filter(|p| !p.is_empty()).collect();
    let mut acc = String::new();
    for dir in &parts[..parts.len().saturating_sub(1)] {
        acc.push('/');
        acc.push_str(dir);
        let _ = client.mkdir(&acc, SmbMode::from(0o755u32));
    }

    let remote = format!("/{}", remote_key.trim_start_matches('/'));
    let mut reader =
        std::fs::File::open(local).with_context(|| format!("open {}", local.display()))?;
    let mut writer = client
        .open_with(
            &remote,
            SmbOpenOptions::default()
                .create(true)
                .write(true)
                .truncate(true),
        )
        .with_context(|| format!("smb open {remote}"))?;
    io::copy(&mut reader, &mut writer).context("smb write")?;
    Ok(())
}

#[async_trait::async_trait]
impl StorageBackend for SmbBackend {
    async fn upload(&self, local_path: &Path, remote_key: &str) -> Result<()> {
        let cfg = self.cfg.clone();
        let local = local_path.to_path_buf();
        let remote = remote_key.to_string();
        tokio::task::spawn_blocking(move || upload_blocking(&cfg, &local, &remote))
            .await
            .context("smb upload task")?
    }

    async fn test(&self) -> Result<()> {
        let cfg = self.cfg.clone();
        tokio::task::spawn_blocking(move || client(&cfg).map(|_| ()))
            .await
            .context("smb test task")?
    }
}
