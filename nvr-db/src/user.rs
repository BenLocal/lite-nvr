use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use turso::Connection;

use crate::kv;

/// KV module namespace user records live under (`kvs.module`, keyed by username).
const MODULE: &str = "user";

#[derive(Debug, Serialize, Deserialize)]
pub struct UserInfo {
    pub username: String,
    pub password_hash: String,
    pub metadata: HashMap<String, String>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

/// Look up a user by username. `Ok(None)` means no such user; a stored value
/// that fails to parse surfaces as an error.
pub async fn get_by_username(
    username: &str,
    conn: &Connection,
) -> anyhow::Result<Option<UserInfo>> {
    match kv::by_module_and_key(MODULE, username, conn).await? {
        Some(kv) => {
            let user = serde_json::from_str(&kv.value.unwrap_or_default())?;
            Ok(Some(user))
        }
        None => Ok(None),
    }
}

/// Whether a user with `username` exists.
pub async fn exists(username: &str, conn: &Connection) -> anyhow::Result<bool> {
    Ok(kv::by_module_and_key(MODULE, username, conn)
        .await?
        .is_some())
}

/// Insert a new user record.
pub async fn insert(user: &UserInfo, conn: &Connection) -> anyhow::Result<()> {
    let value = serde_json::to_string(user)?;
    conn.execute(
        "INSERT INTO kvs (module, key, sub_key, value) VALUES (?1, ?2, ?3, ?4)",
        (MODULE, user.username.as_str(), "", value.as_str()),
    )
    .await?;
    Ok(())
}
