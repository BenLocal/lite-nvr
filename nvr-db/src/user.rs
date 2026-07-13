use std::collections::HashMap;

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
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

/// Overwrite an existing user record (keyed by username).
pub async fn update(user: &UserInfo, conn: &Connection) -> anyhow::Result<()> {
    let value = serde_json::to_string(user)?;
    conn.execute(
        "UPDATE kvs SET value = ?1 WHERE module = ?2 AND key = ?3",
        (value.as_str(), MODULE, user.username.as_str()),
    )
    .await?;
    Ok(())
}

/// Delete a user by username. Deleting a missing user is not an error.
pub async fn delete(username: &str, conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM kvs WHERE module = ?1 AND key = ?2",
        (MODULE, username),
    )
    .await?;
    Ok(())
}

/// List all user records.
pub async fn list(conn: &Connection) -> anyhow::Result<Vec<UserInfo>> {
    kv::by_module(MODULE, conn)
        .await?
        .into_iter()
        .map(|kv| Ok(serde_json::from_str(&kv.value.unwrap_or_default())?))
        .collect()
}

/// Hash a plaintext password with argon2 and a random salt.
pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::encode_b64(uuid::Uuid::new_v4().as_bytes())
        .map_err(|e| anyhow::anyhow!("Failed to generate password salt: {}", e))?;
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("Failed to hash password: {}", e))?;
    Ok(hash.to_string())
}

/// Verify a plaintext password against a stored argon2 hash. An unparsable
/// hash counts as a mismatch rather than an error.
pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
#[path = "user_test.rs"]
mod user_test;
