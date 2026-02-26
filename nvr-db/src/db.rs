use turso::{Builder, Connection, Database};

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

    pub fn connect(&self) -> anyhow::Result<Connection> {
        self.db.connect().map_err(anyhow::Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connect() {
        let config = DatabaseConfig { url: ":memory:" };
        let db = NvrDatabase::new(&config).await.unwrap();
        let conn = db.connect();
        assert!(conn.is_ok());
    }
}
