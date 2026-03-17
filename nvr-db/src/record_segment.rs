use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use turso::Connection;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordSegment {
    pub id: String,
    pub record_type: i32,
    pub start_time: u64,
    pub duration: f32,
    pub file_size: usize,
    pub file_name: String,
    pub file_path: String,
    pub folder: String,
    pub app: String,
    pub stream: String,
    pub vhost: String,
    pub video_codec: String,
    pub video_width: i32,
    pub video_height: i32,
    pub video_fps: f32,
    pub video_bit_rate: i64,
    pub audio_codec: String,
    pub audio_sample_rate: i32,
    pub audio_channels: i32,
    pub audio_bit_rate: i64,
    pub reserve_text1: String,
    pub reserve_text2: String,
    pub reserve_text3: String,
    pub reserve_int1: i64,
    pub reserve_int2: i64,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub async fn upsert(record: &RecordSegment, conn: &Connection) -> anyhow::Result<()> {
    let create_time = record.create_time.to_rfc3339();
    let update_time = record.update_time.to_rfc3339();
    let sql = format!(
        r#"
        INSERT INTO record_segments (
            id, record_type, start_time, duration, file_size, file_name, file_path, folder, app, stream, vhost,
            video_codec, video_width, video_height, video_fps, video_bit_rate,
            audio_codec, audio_sample_rate, audio_channels, audio_bit_rate,
            reserve_text1, reserve_text2, reserve_text3, reserve_int1, reserve_int2, create_time, update_time
        ) VALUES (
            '{id}', {record_type}, {start_time}, {duration}, {file_size}, '{file_name}', '{file_path}', '{folder}', '{app}', '{stream}', '{vhost}',
            '{video_codec}', {video_width}, {video_height}, {video_fps}, {video_bit_rate},
            '{audio_codec}', {audio_sample_rate}, {audio_channels}, {audio_bit_rate},
            '{reserve_text1}', '{reserve_text2}', '{reserve_text3}', {reserve_int1}, {reserve_int2}, '{create_time}', '{update_time}'
        )
        ON CONFLICT(file_path) DO UPDATE SET
            record_type=excluded.record_type,
            start_time=excluded.start_time,
            duration=excluded.duration,
            file_size=excluded.file_size,
            file_name=excluded.file_name,
            folder=excluded.folder,
            app=excluded.app,
            stream=excluded.stream,
            vhost=excluded.vhost,
            video_codec=excluded.video_codec,
            video_width=excluded.video_width,
            video_height=excluded.video_height,
            video_fps=excluded.video_fps,
            video_bit_rate=excluded.video_bit_rate,
            audio_codec=excluded.audio_codec,
            audio_sample_rate=excluded.audio_sample_rate,
            audio_channels=excluded.audio_channels,
            audio_bit_rate=excluded.audio_bit_rate,
            reserve_text1=excluded.reserve_text1,
            reserve_text2=excluded.reserve_text2,
            reserve_text3=excluded.reserve_text3,
            reserve_int1=excluded.reserve_int1,
            reserve_int2=excluded.reserve_int2,
            update_time=excluded.update_time
        "#,
        id = sql_text(&record.id),
        record_type = record.record_type,
        start_time = record.start_time,
        duration = record.duration,
        file_size = record.file_size,
        file_name = sql_text(&record.file_name),
        file_path = sql_text(&record.file_path),
        folder = sql_text(&record.folder),
        app = sql_text(&record.app),
        stream = sql_text(&record.stream),
        vhost = sql_text(&record.vhost),
        video_codec = sql_text(&record.video_codec),
        video_width = record.video_width,
        video_height = record.video_height,
        video_fps = record.video_fps,
        video_bit_rate = record.video_bit_rate,
        audio_codec = sql_text(&record.audio_codec),
        audio_sample_rate = record.audio_sample_rate,
        audio_channels = record.audio_channels,
        audio_bit_rate = record.audio_bit_rate,
        reserve_text1 = sql_text(&record.reserve_text1),
        reserve_text2 = sql_text(&record.reserve_text2),
        reserve_text3 = sql_text(&record.reserve_text3),
        reserve_int1 = record.reserve_int1,
        reserve_int2 = record.reserve_int2,
        create_time = sql_text(&create_time),
        update_time = sql_text(&update_time),
    );
    conn.execute_batch(sql).await?;
    Ok(())
}

pub async fn list(conn: &Connection) -> anyhow::Result<Vec<RecordSegment>> {
    let mut rows = conn
        .query(
            r#"
            SELECT
                id, record_type, start_time, duration, file_size, file_name, file_path, folder, app, stream, vhost,
                video_codec, video_width, video_height, video_fps, video_bit_rate,
                audio_codec, audio_sample_rate, audio_channels, audio_bit_rate,
                reserve_text1, reserve_text2, reserve_text3, reserve_int1, reserve_int2, create_time, update_time
            FROM record_segments
            ORDER BY start_time DESC, update_time DESC
            "#,
            (),
        )
        .await?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().await? {
        records.push(record_from_row(&row)?);
    }
    Ok(records)
}

pub async fn list_by_stream(stream: &str, conn: &Connection) -> anyhow::Result<Vec<RecordSegment>> {
    let mut rows = conn
        .query(
            r#"
            SELECT
                id, record_type, start_time, duration, file_size, file_name, file_path, folder, app, stream, vhost,
                video_codec, video_width, video_height, video_fps, video_bit_rate,
                audio_codec, audio_sample_rate, audio_channels, audio_bit_rate,
                reserve_text1, reserve_text2, reserve_text3, reserve_int1, reserve_int2, create_time, update_time
            FROM record_segments
            WHERE stream = ?1
            ORDER BY start_time DESC, update_time DESC
            "#,
            [stream],
        )
        .await?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().await? {
        records.push(record_from_row(&row)?);
    }
    Ok(records)
}

pub async fn list_by_stream_page(
    stream: &str,
    page: usize,
    page_size: usize,
    conn: &Connection,
) -> anyhow::Result<Vec<RecordSegment>> {
    let offset = page.saturating_sub(1) * page_size;
    let mut rows = conn
        .query(
            r#"
            SELECT
                id, record_type, start_time, duration, file_size, file_name, file_path, folder, app, stream, vhost,
                video_codec, video_width, video_height, video_fps, video_bit_rate,
                audio_codec, audio_sample_rate, audio_channels, audio_bit_rate,
                reserve_text1, reserve_text2, reserve_text3, reserve_int1, reserve_int2, create_time, update_time
            FROM record_segments
            WHERE stream = ?1
            ORDER BY start_time DESC, update_time DESC
            LIMIT ?2 OFFSET ?3
            "#,
            (stream, page_size as i64, offset as i64),
        )
        .await?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().await? {
        records.push(record_from_row(&row)?);
    }
    Ok(records)
}

pub async fn list_by_stream_time_range(
    stream: &str,
    start_time: u64,
    end_time: u64,
    conn: &Connection,
) -> anyhow::Result<Vec<RecordSegment>> {
    let mut rows = conn
        .query(
            r#"
            SELECT
                id, record_type, start_time, duration, file_size, file_name, file_path, folder, app, stream, vhost,
                video_codec, video_width, video_height, video_fps, video_bit_rate,
                audio_codec, audio_sample_rate, audio_channels, audio_bit_rate,
                reserve_text1, reserve_text2, reserve_text3, reserve_int1, reserve_int2, create_time, update_time
            FROM record_segments
            WHERE stream = ?1 AND start_time >= ?2 AND start_time < ?3
            ORDER BY start_time ASC, update_time ASC
            "#,
            (stream, start_time as i64, end_time as i64),
        )
        .await?;
    let mut records = Vec::new();
    while let Some(row) = rows.next().await? {
        records.push(record_from_row(&row)?);
    }
    Ok(records)
}

pub async fn count_by_stream(stream: &str, conn: &Connection) -> anyhow::Result<usize> {
    let mut rows = conn
        .query(
            "SELECT COUNT(*) FROM record_segments WHERE stream = ?1",
            [stream],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(0);
    };
    Ok(row.get::<i64>(0)? as usize)
}

pub async fn count_by_streams(
    streams: &[String],
    conn: &Connection,
) -> anyhow::Result<std::collections::HashMap<String, usize>> {
    use std::collections::HashMap;

    if streams.is_empty() {
        return Ok(HashMap::new());
    }

    let in_clause = streams
        .iter()
        .map(|stream| format!("'{}'", sql_text(stream)))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT stream, COUNT(*) FROM record_segments WHERE stream IN ({}) GROUP BY stream",
        in_clause
    );
    let mut rows = conn.query(sql, ()).await?;
    let mut result = HashMap::new();
    while let Some(row) = rows.next().await? {
        result.insert(row.get::<String>(0)?, row.get::<i64>(1)? as usize);
    }
    Ok(result)
}

pub async fn get(id: &str, conn: &Connection) -> anyhow::Result<Option<RecordSegment>> {
    let mut rows = conn
        .query(
            r#"
            SELECT
                id, record_type, start_time, duration, file_size, file_name, file_path, folder, app, stream, vhost,
                video_codec, video_width, video_height, video_fps, video_bit_rate,
                audio_codec, audio_sample_rate, audio_channels, audio_bit_rate,
                reserve_text1, reserve_text2, reserve_text3, reserve_int1, reserve_int2, create_time, update_time
            FROM record_segments
            WHERE id = ?1
            LIMIT 1
            "#,
            [id],
        )
        .await?;

    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    Ok(Some(record_from_row(&row)?))
}

fn sql_text(value: &str) -> String {
    value.replace('\'', "''")
}

fn record_from_row(row: &turso::Row) -> anyhow::Result<RecordSegment> {
    let create_time = DateTime::parse_from_rfc3339(&row.get::<String>(25)?)?.with_timezone(&Utc);
    let update_time = DateTime::parse_from_rfc3339(&row.get::<String>(26)?)?.with_timezone(&Utc);
    Ok(RecordSegment {
        id: row.get::<String>(0)?,
        record_type: row.get::<i32>(1)?,
        start_time: row.get::<u64>(2)?,
        duration: row.get::<f64>(3)? as f32,
        file_size: row.get::<i64>(4)? as usize,
        file_name: row.get::<String>(5)?,
        file_path: row.get::<String>(6)?,
        folder: row.get::<String>(7)?,
        app: row.get::<String>(8)?,
        stream: row.get::<String>(9)?,
        vhost: row.get::<String>(10)?,
        video_codec: row.get::<String>(11)?,
        video_width: row.get::<i32>(12)?,
        video_height: row.get::<i32>(13)?,
        video_fps: row.get::<f64>(14)? as f32,
        video_bit_rate: row.get::<i64>(15)?,
        audio_codec: row.get::<String>(16)?,
        audio_sample_rate: row.get::<i32>(17)?,
        audio_channels: row.get::<i32>(18)?,
        audio_bit_rate: row.get::<i64>(19)?,
        reserve_text1: row.get::<String>(20)?,
        reserve_text2: row.get::<String>(21)?,
        reserve_text3: row.get::<String>(22)?,
        reserve_int1: row.get::<i64>(23)?,
        reserve_int2: row.get::<i64>(24)?,
        create_time,
        update_time,
    })
}
