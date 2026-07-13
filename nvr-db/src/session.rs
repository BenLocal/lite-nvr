//! Login sessions, KV-backed like `user`: `module="session"`, keyed by the
//! bearer token, with the username duplicated into `sub_key` so revoking all
//! of a user's sessions is a single SQL delete. The stored value carries the
//! expiry; TTL policy and validation caching live in the `nvr` crate.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use turso::Connection;

use crate::kv;

/// KV module namespace session records live under (`kvs.module`).
const MODULE: &str = "session";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub username: String,
    pub expires_at: DateTime<Utc>,
}

/// Insert a new session record.
pub async fn insert(session: &Session, conn: &Connection) -> anyhow::Result<()> {
    let value = serde_json::to_string(session)?;
    conn.execute(
        "INSERT INTO kvs (module, key, sub_key, value) VALUES (?1, ?2, ?3, ?4)",
        (
            MODULE,
            session.token.as_str(),
            session.username.as_str(),
            value.as_str(),
        ),
    )
    .await?;
    Ok(())
}

/// Look up a session by token. `Ok(None)` means no such session; a stored
/// value that fails to parse surfaces as an error.
pub async fn get_by_token(token: &str, conn: &Connection) -> anyhow::Result<Option<Session>> {
    match kv::by_module_and_key(MODULE, token, conn).await? {
        Some(kv) => Ok(Some(serde_json::from_str(&kv.value.unwrap_or_default())?)),
        None => Ok(None),
    }
}

/// Delete a session by token. Deleting a missing session is not an error.
pub async fn delete(token: &str, conn: &Connection) -> anyhow::Result<()> {
    conn.execute(
        "DELETE FROM kvs WHERE module = ?1 AND key = ?2",
        (MODULE, token),
    )
    .await?;
    Ok(())
}

/// Delete all of a user's sessions, optionally sparing one token (the
/// caller's current session, e.g. on password change).
pub async fn delete_by_username(
    username: &str,
    except_token: Option<&str>,
    conn: &Connection,
) -> anyhow::Result<()> {
    match except_token {
        Some(token) => {
            conn.execute(
                "DELETE FROM kvs WHERE module = ?1 AND sub_key = ?2 AND key != ?3",
                (MODULE, username, token),
            )
            .await?
        }
        None => {
            conn.execute(
                "DELETE FROM kvs WHERE module = ?1 AND sub_key = ?2",
                (MODULE, username),
            )
            .await?
        }
    };
    Ok(())
}

/// Garbage-collect sessions that expired at or before `now`. The expiry lives
/// inside the JSON value, and sessions are few, so filter in code instead of
/// relying on JSON SQL functions. An unparsable record is deleted too.
pub async fn delete_expired(now: DateTime<Utc>, conn: &Connection) -> anyhow::Result<()> {
    for kv in kv::by_module(MODULE, conn).await? {
        let expired = match serde_json::from_str::<Session>(&kv.value.unwrap_or_default()) {
            Ok(session) => session.expires_at <= now,
            Err(_) => true,
        };
        if expired {
            delete(&kv.key, conn).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "session_test.rs"]
mod session_test;
