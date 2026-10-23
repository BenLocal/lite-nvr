use std::time::Duration;

use turso::{Builder, Connection, Database};

/// How long a connection waits for another writer's lock before giving up.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub struct DatabaseConfig<'a> {
    url: &'a str,
}

impl<'a> DatabaseConfig<'a> {
    pub fn new(url: &'a str) -> Self {
        Self { url }
    }
}

pub struct NvrDatabase {
    db: Database,
}

impl NvrDatabase {
    pub async fn new(config: &DatabaseConfig<'_>) -> anyhow::Result<Self> {
        let db = Builder::new_local(config.url).build().await?;

        // Enable WAL mode for better performance
        let conn = db.connect().map_err(anyhow::Error::from)?;
        conn.pragma_update("journal_mode", "wal").await?;

        Ok(Self { db })
    }

    /// WAL still allows one writer at a time, and turso's connections default to
    /// no busy handler, so a writer that meets another's lock fails at once with
    /// `database is locked`. Wait for the lock instead; turso backs off without
    /// blocking the async runtime.
    pub fn connect(&self) -> anyhow::Result<Connection> {
        let conn = self.db.connect().map_err(anyhow::Error::from)?;
        conn.busy_timeout(BUSY_TIMEOUT)?;
        Ok(conn)
    }
}

#[cfg(test)]
#[path = "db_test.rs"]
mod db_test;
