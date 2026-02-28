use serde::{Deserialize, Serialize};
use turso::Connection;

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
pub struct Device {
    pub id: i64,
    pub name: String,
    pub status: String,
    pub input: serde_json::Value,
    pub outputs: Vec<serde_json::Value>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeviceCreate {
    pub name: String,
    pub input: serde_json::Value,
    pub outputs: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct DeviceUpdate {
    pub name: Option<String>,
    pub input: Option<serde_json::Value>,
    pub outputs: Option<Vec<serde_json::Value>>,
}

const MODULE_NAME: &str = "device";

pub async fn query_all(conn: &Connection) -> anyhow::Result<Vec<Device>> {
    let kvs = crate::kv::by_module(MODULE_NAME, conn).await?;
    let mut devices = Vec::new();
    for kv in kvs {
        if let Some(json) = kv.value {
            if let Ok(mut device) = serde_json::from_str::<Device>(&json) {
                // Ensure the `id` from the KVs table matches the device's id property for consistency
                device.id = kv.id;
                devices.push(device);
            }
        }
    }
    // Sort by id descending
    devices.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(devices)
}

pub async fn insert(create: &DeviceCreate, conn: &Connection) -> anyhow::Result<Device> {
    // Check if device with same name already exists
    if let Some(_) = crate::kv::by_module_and_key(MODULE_NAME, &create.name, conn).await? {
        return Err(anyhow::anyhow!("Device with name '{}' already exists", create.name));
    }

    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let device = Device {
        id: 0, // Will be replaced after DB insert
        name: create.name.clone(),
        status: "offline".to_string(),
        input: create.input.clone(),
        outputs: create.outputs.clone(),
        created_at: Some(now.clone()),
        updated_at: Some(now),
    };

    let value = serde_json::to_string(&device)?;
    
    conn.execute(
        "INSERT INTO kvs (module, key, sub_key, value) VALUES (?1, ?2, '', ?3)",
        (MODULE_NAME, create.name.as_str(), value.as_str()),
    )
    .await?;

    let last_id = conn.last_insert_rowid();
    by_id(last_id, conn).await?.ok_or_else(|| anyhow::anyhow!("Insert failed, device not found"))
}

pub async fn by_id(id: i64, conn: &Connection) -> anyhow::Result<Option<Device>> {
    let kv = crate::kv::by_id(id, conn).await?;
    if let Some(kv) = kv {
        if kv.module == MODULE_NAME {
            if let Some(json) = kv.value {
                let mut device: Device = serde_json::from_str(&json)?;
                device.id = kv.id;
                return Ok(Some(device));
            }
        }
    }
    Ok(None)
}

pub async fn update(id: i64, update: &DeviceUpdate, conn: &Connection) -> anyhow::Result<Option<Device>> {
    let mut current = match by_id(id, conn).await? {
        Some(d) => d,
        None => return Ok(None),
    };

    let mut key_changed = false;

    if let Some(name) = &update.name {
        current.name = name.clone();
        key_changed = true;
    }
    if let Some(input) = &update.input {
        current.input = input.clone();
    }
    if let Some(outputs) = &update.outputs {
        current.outputs = outputs.clone();
    }

    current.updated_at = Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    
    let json_value = serde_json::to_string(&current)?;

    if key_changed {
        conn.execute(
            "UPDATE kvs SET key = ?1, value = ?2 WHERE id = ?3 AND module = ?4",
            (current.name.as_str(), json_value.as_str(), id, MODULE_NAME),
        )
        .await?;
    } else {
        conn.execute(
            "UPDATE kvs SET value = ?1 WHERE id = ?2 AND module = ?3",
            (json_value.as_str(), id, MODULE_NAME),
        )
        .await?;
    }

    by_id(id, conn).await
}

pub async fn update_status(id: i64, status: &str, conn: &Connection) -> anyhow::Result<()> {
    let mut current = match by_id(id, conn).await? {
        Some(d) => d,
        None => return Ok(()),
    };

    current.status = status.to_string();
    current.updated_at = Some(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    
    let json_value = serde_json::to_string(&current)?;

    conn.execute(
        "UPDATE kvs SET value = ?1 WHERE id = ?2 AND module = ?3",
        (json_value.as_str(), id, MODULE_NAME),
    )
    .await?;

    Ok(())
}

pub async fn delete(id: i64, conn: &Connection) -> anyhow::Result<bool> {
    let affected = conn
        .execute(
            "DELETE FROM kvs WHERE id = ?1 AND module = ?2",
            (id, MODULE_NAME),
        )
        .await?;
    Ok(affected > 0)
}
