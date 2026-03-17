use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use turso::Connection;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub input_type: String,
    pub input_value: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn list(conn: &Connection) -> anyhow::Result<Vec<DeviceInfo>> {
    let kvs = crate::kv::by_module("device", conn).await?;
    let mut devices = kvs
        .into_iter()
        .filter_map(|kv| kv.value)
        .map(|value| serde_json::from_str::<DeviceInfo>(&value))
        .collect::<Result<Vec<_>, _>>()?;
    devices.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(devices)
}

pub async fn count(conn: &Connection) -> anyhow::Result<usize> {
    let mut rows = conn
        .query("SELECT COUNT(*) FROM kvs WHERE module = ?1", ["device"])
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(0);
    };
    Ok(row.get::<i64>(0)? as usize)
}

pub async fn list_page(
    page: usize,
    page_size: usize,
    conn: &Connection,
) -> anyhow::Result<Vec<DeviceInfo>> {
    let offset = page.saturating_sub(1) * page_size;
    let mut rows = conn
        .query(
            r#"
            SELECT value
            FROM kvs
            WHERE module = ?1 AND value IS NOT NULL
            ORDER BY json_extract(value, '$.updated_at') DESC
            LIMIT ?2 OFFSET ?3
            "#,
            ("device", page_size as i64, offset as i64),
        )
        .await?;

    let mut devices = Vec::new();
    while let Some(row) = rows.next().await? {
        devices.push(serde_json::from_str::<DeviceInfo>(&row.get::<String>(0)?)?);
    }
    Ok(devices)
}

pub async fn get(id: &str, conn: &Connection) -> anyhow::Result<Option<DeviceInfo>> {
    let kv = crate::kv::by_module_and_key("device", id, conn).await?;
    match kv.and_then(|item| item.value) {
        Some(value) => Ok(Some(serde_json::from_str::<DeviceInfo>(&value)?)),
        None => Ok(None),
    }
}

pub async fn upsert(device: &DeviceInfo, conn: &Connection) -> anyhow::Result<()> {
    let value = serde_json::to_string(device)?;
    if crate::kv::by_module_and_key("device", &device.id, conn)
        .await?
        .is_some()
    {
        conn.execute(
            "UPDATE kvs SET value = ?1, sub_key = ?2 WHERE module = ?3 AND key = ?4",
            (value.as_str(), "", "device", device.id.as_str()),
        )
        .await?;
    } else {
        conn.execute(
            "INSERT INTO kvs (module, key, sub_key, value) VALUES (?1, ?2, ?3, ?4)",
            ("device", device.id.as_str(), "", value.as_str()),
        )
        .await?;
    }

    Ok(())
}

pub async fn delete(id: &str, conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM kvs WHERE module = ?1 AND key = ?2",
        ("device", id),
    )
    .await?;
    Ok(())
}
