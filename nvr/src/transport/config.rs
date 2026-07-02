//! Kind-specific transport settings, parsed from a `TransportTarget.config`
//! JSON blob, plus the remote object-key layout.

use nvr_db::record_segment::RecordSegment;
use nvr_db::transport_target::TransportTarget;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtpConfig {
    pub host: String,
    #[serde(default = "default_ftp_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// Directory prefix on the server (e.g. "nvr/records"); may be empty.
    #[serde(default)]
    pub base_path: String,
}

fn default_ftp_port() -> u16 {
    21
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmbConfig {
    /// Server host or `smb://host`.
    pub host: String,
    /// Share name (e.g. "records").
    pub share: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_workgroup")]
    pub workgroup: String,
    #[serde(default)]
    pub base_path: String,
}

fn default_workgroup() -> String {
    "WORKGROUP".to_string()
}

/// Build the remote key `[base_path/]<stream>/<file_name>` with clean slashes.
pub fn remote_key(base_path: &str, stream: &str, file_name: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let base = base_path.trim_matches('/');
    if !base.is_empty() {
        parts.push(base);
    }
    let stream = stream.trim_matches('/');
    if !stream.is_empty() {
        parts.push(stream);
    }
    parts.push(file_name);
    parts.join("/")
}

/// The remote key a given segment should be uploaded to for `target`.
pub fn remote_key_for(target: &TransportTarget, segment: &RecordSegment) -> String {
    remote_key(&base_path_of(target), &segment.stream, &segment.file_name)
}

fn base_path_of(target: &TransportTarget) -> String {
    match target.kind.as_str() {
        "ftp" => serde_json::from_str::<FtpConfig>(&target.config)
            .map(|c| c.base_path)
            .unwrap_or_default(),
        "smb" => serde_json::from_str::<SmbConfig>(&target.config)
            .map(|c| c.base_path)
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Return `config` with any `password` field blanked, for safe display in API
/// responses (the real value stays in the DB).
pub fn redact_config(config: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(config) {
        Ok(mut value) => {
            if let Some(obj) = value.as_object_mut() {
                if obj.contains_key("password") {
                    obj.insert(
                        "password".to_string(),
                        serde_json::Value::String(String::new()),
                    );
                }
            }
            value.to_string()
        }
        Err(_) => config.to_string(),
    }
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
