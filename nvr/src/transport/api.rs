//! REST API for managing transport targets and inspecting upload jobs. GET/POST
//! only (the dashboard convention). Passwords are redacted in responses and
//! preserved on update when the client sends a blank one.

use axum::{
    Json, Router,
    extract::Path,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use nvr_db::transport_job::{self, TransportJob};
use nvr_db::transport_target::{self, TransportTarget};

use crate::db::app_db_conn;
use crate::handler::{ApiJsonResult, ok_empty, ok_json};
use crate::transport::backend::build_backend;
use crate::transport::config::redact_config;

pub fn transport_router() -> Router {
    Router::new()
        .route("/targets", get(list_targets))
        .route("/target/add", post(add_target))
        .route("/target/update/{id}", post(update_target))
        .route("/target/remove/{id}", post(remove_target))
        .route("/target/test/{id}", post(test_target))
        .route("/jobs/{target_id}", get(list_jobs))
}

#[derive(Deserialize)]
struct TargetPayload {
    name: String,
    kind: String,
    #[serde(default = "default_true")]
    enabled: bool,
    /// Kind-specific settings as a JSON object (host, credentials, base_path…).
    config: serde_json::Value,
    #[serde(default)]
    remark: String,
}

fn default_true() -> bool {
    true
}

#[derive(Serialize)]
struct TargetDto {
    id: String,
    name: String,
    kind: String,
    enabled: bool,
    config: serde_json::Value,
    remark: String,
    create_time: String,
    update_time: String,
    done: i64,
    failed: i64,
    pending: i64,
}

fn to_dto(target: TransportTarget, done: i64, failed: i64, pending: i64) -> TargetDto {
    let config =
        serde_json::from_str(&redact_config(&target.config)).unwrap_or(serde_json::Value::Null);
    TargetDto {
        id: target.id,
        name: target.name,
        kind: target.kind,
        enabled: target.enabled,
        config,
        remark: target.remark,
        create_time: target.create_time,
        update_time: target.update_time,
        done,
        failed,
        pending,
    }
}

fn validate_kind(kind: &str) -> anyhow::Result<()> {
    match kind {
        "ftp" | "smb" => Ok(()),
        other => anyhow::bail!("unsupported transport kind: {other}"),
    }
}

async fn list_targets() -> ApiJsonResult<Vec<TargetDto>> {
    let conn = app_db_conn()?;
    let targets = transport_target::list(&conn).await?;
    let mut out = Vec::with_capacity(targets.len());
    for target in targets {
        let (done, failed, pending) = transport_job::counts_by_status(&target.id, &conn).await?;
        out.push(to_dto(target, done, failed, pending));
    }
    Ok(ok_json(out))
}

async fn add_target(Json(payload): Json<TargetPayload>) -> ApiJsonResult<TargetDto> {
    validate_kind(payload.kind.trim())?;
    let conn = app_db_conn()?;
    let now = chrono::Utc::now().to_rfc3339();
    let target = TransportTarget {
        id: uuid::Uuid::new_v4().simple().to_string(),
        name: payload.name.trim().to_string(),
        kind: payload.kind.trim().to_string(),
        enabled: payload.enabled,
        config: payload.config.to_string(),
        remark: payload.remark,
        create_time: now.clone(),
        update_time: now,
    };
    transport_target::upsert(&target, &conn).await?;
    Ok(ok_json(to_dto(target, 0, 0, 0)))
}

async fn update_target(
    Path(id): Path<String>,
    Json(payload): Json<TargetPayload>,
) -> ApiJsonResult<TargetDto> {
    validate_kind(payload.kind.trim())?;
    let conn = app_db_conn()?;
    let existing = transport_target::get(&id, &conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("transport target not found"))?;
    let config = merge_password(&existing.config, payload.config);
    let now = chrono::Utc::now().to_rfc3339();
    let target = TransportTarget {
        id: existing.id,
        name: payload.name.trim().to_string(),
        kind: payload.kind.trim().to_string(),
        enabled: payload.enabled,
        config,
        remark: payload.remark,
        create_time: existing.create_time,
        update_time: now,
    };
    transport_target::upsert(&target, &conn).await?;
    let (done, failed, pending) = transport_job::counts_by_status(&target.id, &conn).await?;
    Ok(ok_json(to_dto(target, done, failed, pending)))
}

/// Keep the stored password when the client sends a blank one (the redacted
/// value round-tripped from a GET).
fn merge_password(existing_config: &str, mut incoming: serde_json::Value) -> String {
    if let Some(obj) = incoming.as_object_mut() {
        let blank = obj
            .get("password")
            .and_then(|value| value.as_str())
            .map(str::is_empty)
            .unwrap_or(true);
        if blank {
            if let Ok(old) = serde_json::from_str::<serde_json::Value>(existing_config) {
                if let Some(password) = old.get("password") {
                    obj.insert("password".to_string(), password.clone());
                }
            }
        }
    }
    incoming.to_string()
}

async fn remove_target(Path(id): Path<String>) -> ApiJsonResult<()> {
    let conn = app_db_conn()?;
    transport_target::delete(&id, &conn).await?;
    Ok(ok_empty())
}

/// Test connectivity/auth of a saved target using its stored (real) credentials.
async fn test_target(Path(id): Path<String>) -> ApiJsonResult<()> {
    let conn = app_db_conn()?;
    let target = transport_target::get(&id, &conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("transport target not found"))?;
    let backend = build_backend(&target)?;
    backend.test().await?;
    Ok(ok_empty())
}

async fn list_jobs(Path(target_id): Path<String>) -> ApiJsonResult<Vec<TransportJob>> {
    let conn = app_db_conn()?;
    let jobs = transport_job::list_recent(&target_id, 50, &conn).await?;
    Ok(ok_json(jobs))
}
