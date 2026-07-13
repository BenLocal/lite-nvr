//! Session auth: token issuance/validation backed by the session KV store
//! (`nvr_db::session`) plus the axum middleware guarding the `/api` router.
//!
//! Validation goes through a process-wide cache because `app_db_conn`
//! deliberately opens a fresh turso connection per call (see `crate::db`);
//! paying that on every request would be wasteful. A cache miss falls back to
//! the DB, so sessions survive process restarts.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use axum::{
    Json,
    extract::Request,
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::{DateTime, Duration, Utc};

use crate::db::app_db_conn;
use crate::handler::BaseResponse;

/// Sessions live this long from login. Fixed, not sliding — renewal would
/// cost a DB write per request.
const SESSION_TTL_DAYS: i64 = 30;

/// Paths (relative to the `/api` router the middleware is layered on, which
/// sees the nest-stripped URI) that skip auth.
const EXEMPT_PATHS: &[&str] = &["/user/login"];

/// The authenticated caller, inserted into request extensions by
/// [`require_auth`]; handlers take it via `Extension<AuthUser>`.
#[derive(Clone)]
pub struct AuthUser {
    pub username: String,
    pub token: String,
}

#[derive(Clone)]
struct CachedSession {
    username: String,
    expires_at: DateTime<Utc>,
}

static CACHE: LazyLock<RwLock<HashMap<String, CachedSession>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Issue a new session token for `username` (DB + cache).
pub async fn create_session(username: &str) -> anyhow::Result<String> {
    let token = uuid::Uuid::new_v4().to_string();
    let expires_at = Utc::now() + Duration::days(SESSION_TTL_DAYS);
    let session = nvr_db::session::Session {
        token: token.clone(),
        username: username.to_string(),
        expires_at,
    };
    nvr_db::session::insert(&session, &app_db_conn()?).await?;
    CACHE.write().unwrap().insert(
        token.clone(),
        CachedSession {
            username: username.to_string(),
            expires_at,
        },
    );
    Ok(token)
}

/// Resolve a token to its username, or `None` if unknown or expired. Expired
/// sessions are deleted as a side effect.
pub async fn validate(token: &str) -> Option<String> {
    let now = Utc::now();
    let cached = CACHE.read().unwrap().get(token).cloned();
    if let Some(cached) = cached {
        if cached.expires_at > now {
            return Some(cached.username);
        }
        let _ = revoke(token).await;
        return None;
    }

    let conn = app_db_conn().ok()?;
    let session = nvr_db::session::get_by_token(token, &conn).await.ok()??;
    if session.expires_at <= now {
        let _ = nvr_db::session::delete(token, &conn).await;
        return None;
    }
    CACHE.write().unwrap().insert(
        token.to_string(),
        CachedSession {
            username: session.username.clone(),
            expires_at: session.expires_at,
        },
    );
    Some(session.username)
}

/// Revoke one session token (DB + cache).
pub async fn revoke(token: &str) -> anyhow::Result<()> {
    CACHE.write().unwrap().remove(token);
    nvr_db::session::delete(token, &app_db_conn()?).await
}

/// Revoke all of a user's sessions, optionally sparing one token (the
/// caller's own, e.g. on password change).
pub async fn revoke_user(username: &str, except_token: Option<&str>) -> anyhow::Result<()> {
    CACHE
        .write()
        .unwrap()
        .retain(|token, s| s.username != username || Some(token.as_str()) == except_token);
    nvr_db::session::delete_by_username(username, except_token, &app_db_conn()?).await
}

/// Middleware guarding the `/api` router. Accepts `Authorization: Bearer` or
/// a `?token=` query param (hls.js / Safari-native playback can't always set
/// headers), exempts login, and stamps the request with [`AuthUser`].
pub async fn require_auth(mut req: Request, next: Next) -> Response {
    if EXEMPT_PATHS.contains(&req.uri().path()) {
        return next.run(req).await;
    }

    let token = bearer_token(&req).or_else(|| query_token(req.uri().query()));
    let Some(token) = token else {
        return unauthorized();
    };
    let Some(username) = validate(&token).await else {
        return unauthorized();
    };

    req.extensions_mut().insert(AuthUser { username, token });
    next.run(req).await
}

fn bearer_token(req: &Request) -> Option<String> {
    let value = req.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

fn query_token(query: Option<&str>) -> Option<String> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix("token="))
        .map(str::to_string)
        .filter(|t| !t.is_empty())
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(BaseResponse::<()> {
            code: 401,
            message: "unauthorized".to_string(),
            data: None,
        }),
    )
        .into_response()
}

#[cfg(test)]
#[path = "auth_test.rs"]
mod auth_test;
