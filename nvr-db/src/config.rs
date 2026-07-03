use serde::Serialize;
use serde::de::DeserializeOwned;
use turso::Connection;

use crate::kv;

/// KV module namespace configuration values live under (`kvs.module`, keyed by
/// config key; `sub_key` is unused).
const MODULE: &str = "config";

/// Fetch a raw config value by key. `Ok(None)` means the key is unset.
pub async fn get(key: &str, conn: &Connection) -> anyhow::Result<Option<String>> {
    Ok(kv::by_module_and_key(MODULE, key, conn)
        .await?
        .and_then(|kv| kv.value))
}

/// Fetch and JSON-deserialize a config value by key.
pub async fn get_json<T: DeserializeOwned>(
    key: &str,
    conn: &Connection,
) -> anyhow::Result<Option<T>> {
    match get(key, conn).await? {
        Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        None => Ok(None),
    }
}

/// Whether a config key is set.
pub async fn exists(key: &str, conn: &Connection) -> anyhow::Result<bool> {
    Ok(kv::by_module_and_key(MODULE, key, conn).await?.is_some())
}

/// Set a config value, overwriting any existing entry for `key`. The `kvs`
/// table has no unique constraint on (module, key), so upsert by hand: update
/// in place, and insert only when nothing was updated.
pub async fn set(key: &str, value: &str, conn: &Connection) -> anyhow::Result<()> {
    let updated = conn
        .execute(
            "UPDATE kvs SET value = ?3 WHERE module = ?1 AND key = ?2",
            (MODULE, key, value),
        )
        .await?;
    if updated == 0 {
        conn.execute(
            "INSERT INTO kvs (module, key, sub_key, value) VALUES (?1, ?2, ?3, ?4)",
            (MODULE, key, "", value),
        )
        .await?;
    }
    Ok(())
}

/// Set a config value from a serializable object, stored as JSON.
pub async fn set_json<T: Serialize>(key: &str, value: &T, conn: &Connection) -> anyhow::Result<()> {
    let raw = serde_json::to_string(value)?;
    set(key, &raw, conn).await
}

/// Delete a config key.
pub async fn delete(key: &str, conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM kvs WHERE module = ?1 AND key = ?2",
        (MODULE, key),
    )
    .await?;
    Ok(())
}
