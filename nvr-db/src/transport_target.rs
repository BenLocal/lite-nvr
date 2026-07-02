use serde::{Deserialize, Serialize};
use turso::Connection;

/// A remote storage destination recorded segments are copied to. `config` is a
/// kind-specific JSON blob (host/credentials/base path…), so adding a backend
/// (e.g. S3) needs no schema change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportTarget {
    pub id: String,
    pub name: String,
    /// "ftp" | "smb" | "s3" …
    pub kind: String,
    pub enabled: bool,
    pub config: String,
    pub remark: String,
    pub create_time: String,
    pub update_time: String,
}

const COLS: &str = "id, name, kind, enabled, config, remark, create_time, update_time";

fn sql_text(value: &str) -> String {
    value.replace('\'', "''")
}

fn from_row(row: &turso::Row) -> anyhow::Result<TransportTarget> {
    Ok(TransportTarget {
        id: row.get::<String>(0)?,
        name: row.get::<String>(1)?,
        kind: row.get::<String>(2)?,
        enabled: row.get::<i64>(3)? != 0,
        config: row.get::<String>(4)?,
        remark: row.get::<String>(5)?,
        create_time: row.get::<String>(6)?,
        update_time: row.get::<String>(7)?,
    })
}

pub async fn list(conn: &Connection) -> anyhow::Result<Vec<TransportTarget>> {
    let sql = format!("SELECT {COLS} FROM transport_targets ORDER BY create_time ASC");
    let mut rows = conn.query(&sql, ()).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(from_row(&row)?);
    }
    Ok(out)
}

pub async fn list_enabled(conn: &Connection) -> anyhow::Result<Vec<TransportTarget>> {
    let sql =
        format!("SELECT {COLS} FROM transport_targets WHERE enabled = 1 ORDER BY create_time ASC");
    let mut rows = conn.query(&sql, ()).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(from_row(&row)?);
    }
    Ok(out)
}

pub async fn get(id: &str, conn: &Connection) -> anyhow::Result<Option<TransportTarget>> {
    let sql = format!("SELECT {COLS} FROM transport_targets WHERE id = ?1 LIMIT 1");
    let mut rows = conn.query(&sql, [id]).await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    Ok(Some(from_row(&row)?))
}

pub async fn upsert(target: &TransportTarget, conn: &Connection) -> anyhow::Result<()> {
    let sql = format!(
        r#"
        INSERT INTO transport_targets (id, name, kind, enabled, config, remark, create_time, update_time)
        VALUES ('{id}', '{name}', '{kind}', {enabled}, '{config}', '{remark}', '{create_time}', '{update_time}')
        ON CONFLICT(id) DO UPDATE SET
            name=excluded.name,
            kind=excluded.kind,
            enabled=excluded.enabled,
            config=excluded.config,
            remark=excluded.remark,
            update_time=excluded.update_time
        "#,
        id = sql_text(&target.id),
        name = sql_text(&target.name),
        kind = sql_text(&target.kind),
        enabled = if target.enabled { 1 } else { 0 },
        config = sql_text(&target.config),
        remark = sql_text(&target.remark),
        create_time = sql_text(&target.create_time),
        update_time = sql_text(&target.update_time),
    );
    conn.execute_batch(sql).await?;
    Ok(())
}

/// Delete a target and all of its transport bookkeeping.
pub async fn delete(id: &str, conn: &Connection) -> anyhow::Result<()> {
    conn.execute("DELETE FROM transport_targets WHERE id = ?1", [id])
        .await?;
    conn.execute("DELETE FROM transport_jobs WHERE target_id = ?1", [id])
        .await?;
    Ok(())
}
