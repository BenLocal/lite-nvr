use std::path::Path;

use crate::db::{DatabaseConfig, database};

const MIGRATIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS _migrations (
    version INTEGER NOT NULL PRIMARY KEY,
    description TEXT NOT NULL,
    createtime TEXT NOT NULL DEFAULT (datetime('now'))
);
"#;

#[derive(Debug, rust_embed::Embed)]
#[folder = "migrations/"]
struct Migrations;

pub async fn migrate(url: &str) -> anyhow::Result<()> {
    let config = DatabaseConfig::new(url);
    let db = database(&config).await;

    let mut conn = db.connect()?;
    ensure_migrations_table(&conn).await?;
    let mut migrations = load_migrations()?;
    migrations.sort_by_key(|m| m.version);

    for migration in migrations {
        if is_migration_applied(&conn, migration.version).await? {
            continue;
        }
        let tx = conn.transaction().await?;
        tx.execute_batch(&migration.sql).await?;
        tx.execute(
            "INSERT INTO _migrations (version, description) VALUES (?1, ?2)",
            (migration.version, migration.description.as_str()),
        )
        .await?;
        tx.commit().await?;
    }

    Ok(())
}

async fn ensure_migrations_table(conn: &turso::Connection) -> anyhow::Result<()> {
    conn.execute_batch(MIGRATIONS_TABLE_SQL).await?;
    Ok(())
}

async fn is_migration_applied(conn: &turso::Connection, version: i64) -> anyhow::Result<bool> {
    let mut rows = conn
        .query("SELECT 1 FROM _migrations WHERE version = ?1", (version,))
        .await?;
    Ok(rows.next().await?.is_some())
}

struct Migration {
    version: i64,
    description: String,
    sql: String,
}

impl Migration {
    fn new(version: i64, description: String, sql: String) -> Self {
        Self {
            version,
            description,
            sql,
        }
    }
}

fn load_migrations() -> anyhow::Result<Vec<Migration>> {
    let mut migrations = Vec::new();
    for path in Migrations::iter() {
        let emb_file = Migrations::get(&path);
        if let Some(emb_file) = emb_file {
            let parts = Path::new(path.as_ref())
                .file_name()
                .map_or("", |x| x.to_str().unwrap_or(""))
                .splitn(2, '_')
                .collect::<Vec<_>>();

            if parts.len() != 2 || !parts[1].ends_with(".sql") {
                // not of the format: <VERSION>_<DESCRIPTION>.sql; ignore
                continue;
            }
            let version: i64 = parts[0].parse()?;

            let description = parts[1]
                .trim_end_matches(".sql")
                .replace('_', " ")
                .to_owned();

            let sql = unsafe { std::str::from_utf8_unchecked(emb_file.data.as_ref()) };

            migrations.push(Migration::new(version, description, sql.to_owned()));
        }
    }
    Ok(migrations)
}
