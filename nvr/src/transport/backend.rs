//! The storage-backend abstraction. New destinations (S3, …) implement
//! `StorageBackend` and get wired into `build_backend`.

use std::path::Path;

use anyhow::Result;
use nvr_db::transport_target::TransportTarget;

use crate::transport::ftp::FtpBackend;
use crate::transport::smb::SmbBackend;

/// A remote storage destination recorded segments can be copied to. Backends run
/// their (blocking) client work on the blocking pool, so the methods are async.
#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync {
    /// Copy the local file to `remote_key`, creating parent directories as needed.
    async fn upload(&self, local_path: &Path, remote_key: &str) -> Result<()>;
    /// Cheap connectivity/auth check (connect + tear down).
    async fn test(&self) -> Result<()>;
}

/// Construct the backend for a target from its `kind` + `config` JSON.
pub fn build_backend(target: &TransportTarget) -> Result<Box<dyn StorageBackend>> {
    match target.kind.as_str() {
        "ftp" => Ok(Box::new(FtpBackend::from_json(&target.config)?)),
        "smb" => Ok(Box::new(SmbBackend::from_json(&target.config)?)),
        other => anyhow::bail!("unsupported transport kind: {other}"),
    }
}
