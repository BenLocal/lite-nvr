use std::sync::LazyLock;

pub struct NvrConfig {
    db_url: String,
}

impl NvrConfig {
    pub fn new(db_url: &str) -> Self {
        Self {
            db_url: db_url.to_string(),
        }
    }

    pub fn db_url(&self) -> &str {
        &self.db_url
    }
}

pub fn config() -> &'static NvrConfig {
    static CONFIG: LazyLock<NvrConfig> = LazyLock::new(|| NvrConfig::new("nvr.db"));
    &CONFIG
}
