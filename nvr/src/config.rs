use std::path::PathBuf;
use std::sync::LazyLock;

pub struct NvrConfig {
    db_url: String,
    /// Optional override for the recording archive directory. `None` falls back
    /// to the default `<cwd>/data/records`.
    record_dir: Option<String>,
}

impl NvrConfig {
    pub fn new(db_url: &str) -> Self {
        Self {
            db_url: db_url.to_string(),
            record_dir: std::env::var("NVR_RECORD_DIR")
                .ok()
                .map(|dir| dir.trim().to_string())
                .filter(|dir| !dir.is_empty()),
        }
    }

    pub fn db_url(&self) -> &str {
        &self.db_url
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
