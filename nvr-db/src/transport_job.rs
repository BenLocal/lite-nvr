use serde::{Deserialize, Serialize};
use turso::Connection;

pub const STATUS_PENDING: i64 = 0;
pub const STATUS_DONE: i64 = 1;
pub const STATUS_FAILED: i64 = 2;

/// Upload bookkeeping for one (segment, target) pair. Since transport is a copy
/// (local files are kept), this only records whether the remote copy exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportJob {
    pub id: String,
    pub segment_id: String,
    pub target_id: String,
    pub status: i64,
    pub attempts: i64,
    pub remote_key: String,
    pub file_size: i64,
    pub error: String,
    pub create_time: String,
    pub update_time: String,
}

const COLS: &str = "id, segment_id, target_id, status, attempts, remote_key, file_size, error, create_time, update_time";

fn sql_text(value: &str) -> String {
    value.replace('\'', "''")
}

fn from_row(row: &turso::Row) -> anyhow::Result<TransportJob> {
    Ok(TransportJob {
        id: row.get::<String>(0)?,
        segment_id: row.get::<String>(1)?,
        target_id: row.get::<String>(2)?,
        status: row.get::<i64>(3)?,
        attempts: row.get::<i64>(4)?,
        remote_key: row.get::<String>(5)?,
        file_size: row.get::<i64>(6)?,
        error: row.get::<String>(7)?,
        create_time: row.get::<String>(8)?,
        update_time: row.get::<String>(9)?,
    })
}

pub async fn upsert(job: &TransportJob, conn: &Connection) -> anyhow::Result<()> {
    let sql = format!(
        r#"
        INSERT INTO transport_jobs (id, segment_id, target_id, status, attempts, remote_key, file_size, error, create_time, update_time)
        VALUES ('{id}', '{segment_id}', '{target_id}', {status}, {attempts}, '{remote_key}', {file_size}, '{error}', '{create_time}', '{update_time}')
        ON CONFLICT(segment_id, target_id) DO UPDATE SET
            status=excluded.status,
            attempts=excluded.attempts,
            remote_key=excluded.remote_key,
            file_size=excluded.file_size,
            error=excluded.error,
            update_time=excluded.update_time
        "#,
        id = sql_text(&job.id),
        segment_id = sql_text(&job.segment_id),
        target_id = sql_text(&job.target_id),
        status = job.status,
        attempts = job.attempts,
        remote_key = sql_text(&job.remote_key),
        file_size = job.file_size,
        error = sql_text(&job.error),
        create_time = sql_text(&job.create_time),
        update_time = sql_text(&job.update_time),
    );
    conn.execute_batch(sql).await?;
    Ok(())
}

pub async fn get(
    segment_id: &str,
    target_id: &str,
    conn: &Connection,
) -> anyhow::Result<Option<TransportJob>> {
    let sql = format!(
        "SELECT {COLS} FROM transport_jobs WHERE segment_id = ?1 AND target_id = ?2 LIMIT 1"
    );
    let mut rows = conn.query(&sql, [segment_id, target_id]).await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    Ok(Some(from_row(&row)?))
}

/// Most recent jobs (any status) for a target, newest first.
pub async fn list_recent(
    target_id: &str,
    limit: usize,
    conn: &Connection,
) -> anyhow::Result<Vec<TransportJob>> {
    let sql = format!(
        "SELECT {COLS} FROM transport_jobs WHERE target_id = ?1 ORDER BY update_time DESC LIMIT {limit}"
    );
    let mut rows = conn.query(&sql, [target_id]).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(from_row(&row)?);
    }
    Ok(out)
}

/// (done, failed, pending) counts for a target.
pub async fn counts_by_status(
    target_id: &str,
    conn: &Connection,
) -> anyhow::Result<(i64, i64, i64)> {
    let sql = "SELECT status, COUNT(*) FROM transport_jobs WHERE target_id = ?1 GROUP BY status";
    let mut rows = conn.query(sql, [target_id]).await?;
    let (mut done, mut failed, mut pending) = (0i64, 0i64, 0i64);
    while let Some(row) = rows.next().await? {
        let status = row.get::<i64>(0)?;
        let count = row.get::<i64>(1)?;
        match status {
            STATUS_DONE => done = count,
            STATUS_FAILED => failed = count,
            _ => pending += count,
        }
    }
    Ok((done, failed, pending))
}
