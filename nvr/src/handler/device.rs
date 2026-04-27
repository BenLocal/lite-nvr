use axum::{
    Json, Router,
    extract::Path,
    routing::{delete, get, post, put},
};
use chrono::Utc;
use harsh::Harsh;
use nvr_db::device::DeviceInfo;
use serde::{Deserialize, Serialize};

use crate::{
    db::app_db_conn,
    handler::{ApiJsonResult, ok_json},
    init::device::{build_flv_url, ensure_device_pipe},
    manager,
};

fn device_id_from_name(name: &str) -> String {
    let digest = md5::compute(name.trim().as_bytes());
    let source = u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ]);
    Harsh::builder()
        .salt("lite-nvr-device")
        .length(12)
        .build()
        .expect("hashids config should be valid")
        .encode(&[source])
}

pub fn device_router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/list", get(list_devices))
        .route("/add", post(add_device))
        .route("/update/{id}", put(update_device))
        .route("/remove/{id}", delete(remove_device))
}

#[derive(Debug, Serialize, Deserialize)]
struct DevicePayload {
    id: Option<String>,
    name: String,
    input_type: String,
    input_value: String,
    description: Option<String>,
    #[serde(default)]
    include_audio: bool,
}

#[derive(Debug, Serialize)]
struct DeviceListItem {
    #[serde(flatten)]
    device: DeviceInfo,
    flv_url: String,
}

async fn index() -> &'static str {
    "device route!"
}

async fn list_devices() -> ApiJsonResult<Vec<DeviceListItem>> {
    let conn = app_db_conn()?;
    let devices = nvr_db::device::list(&conn).await?;
    Ok(ok_json(
        devices
            .into_iter()
            .map(|device| DeviceListItem {
                flv_url: build_flv_url(&device.id),
                device,
            })
            .collect(),
    ))
}

async fn add_device(Json(payload): Json<DevicePayload>) -> ApiJsonResult<DeviceInfo> {
    let conn = app_db_conn()?;
    let now = Utc::now();
    let name = payload.name.trim().to_string();
    let device = DeviceInfo {
        id: payload.id.unwrap_or_else(|| device_id_from_name(&name)),
        name,
        input_type: payload.input_type.trim().to_string(),
        input_value: payload.input_value.trim().to_string(),
        description: payload.description.unwrap_or_default().trim().to_string(),
        include_audio: payload.include_audio,
        created_at: now,
        updated_at: now,
    };
    validate_device(&device)?;
    nvr_db::device::upsert(&device, &conn).await?;
    ensure_device_pipe(&device).await?;
    Ok(ok_json(device))
}

async fn update_device(
    Path(id): Path<String>,
    Json(payload): Json<DevicePayload>,
) -> ApiJsonResult<DeviceInfo> {
    let conn = app_db_conn()?;
    let existing = nvr_db::device::get(&id, &conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("device not found"))?;
    let device = DeviceInfo {
        id,
        name: payload.name.trim().to_string(),
        input_type: payload.input_type.trim().to_string(),
        input_value: payload.input_value.trim().to_string(),
        description: payload.description.unwrap_or_default().trim().to_string(),
        include_audio: payload.include_audio,
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    validate_device(&device)?;
    nvr_db::device::upsert(&device, &conn).await?;
    ensure_device_pipe(&device).await?;
    Ok(ok_json(device))
}

async fn remove_device(Path(id): Path<String>) -> ApiJsonResult<String> {
    let conn = app_db_conn()?;
    nvr_db::device::delete(&id, &conn).await?;
    manager::remove_pipe(&id).await?;
    Ok(ok_json("success".to_string()))
}

fn validate_device(device: &DeviceInfo) -> anyhow::Result<()> {
    if device.name.is_empty() {
        return Err(anyhow::anyhow!("device name is required"));
    }
    if device.input_type.is_empty() {
        return Err(anyhow::anyhow!("input type is required"));
    }
    if device.input_value.is_empty() {
        return Err(anyhow::anyhow!("input value is required"));
    }
    Ok(())
}
