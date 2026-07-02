use std::path::PathBuf;
use std::sync::LazyLock;

use crate::gb::config::GbConfig;

pub struct NvrConfig {
    db_url: String,
    /// Optional override for the recording archive directory. `None` falls back
    /// to the default `<cwd>/data/records`.
    record_dir: Option<String>,
    /// GB28181 platform config, or `None` when disabled (`NVR_GB_ENABLE != 1`).
    gb: Option<GbConfig>,
}

impl NvrConfig {
    pub fn new(db_url: &str) -> Self {
        Self {
            db_url: db_url.to_string(),
            record_dir: std::env::var("NVR_RECORD_DIR")
                .ok()
                .map(|dir| dir.trim().to_string())
                .filter(|dir| !dir.is_empty()),
            gb: GbConfig::from_env(),
        }
    }

    pub fn db_url(&self) -> &str {
        &self.db_url
    }

    /// GB28181 platform config, or `None` when disabled (`NVR_GB_ENABLE != 1`).
    pub fn gb(&self) -> Option<&GbConfig> {
        self.gb.as_ref()
    }

    /// Root directory where recordings are archived. Set via `NVR_RECORD_DIR`;
    /// when unset, defaults to `<cwd>/data/records`.
    pub fn record_dir(&self) -> PathBuf {
        if let Some(dir) = &self.record_dir {
            return PathBuf::from(dir);
        }
        std::env::current_dir()
            .map(|cwd| cwd.join("data").join("records"))
            .unwrap_or_else(|_| PathBuf::from("data").join("records"))
    }
}

pub fn config() -> &'static NvrConfig {
    static CONFIG: LazyLock<NvrConfig> = LazyLock::new(|| NvrConfig::new("nvr.db"));
    &CONFIG
}
