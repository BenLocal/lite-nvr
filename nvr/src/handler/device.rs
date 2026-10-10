use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use chrono::Utc;
use harsh::Harsh;
use nvr_db::device::{DeviceConfig, DeviceInfo};
use serde::{Deserialize, Serialize};

use crate::{
    db::app_db_conn,
    handler::{ApiJsonResult, ok_json},
    init::device::{build_flv_url, build_gb_flv_url, ensure_device_pipe},
    manager,
};

static DEVICE_OPERATIONS: std::sync::LazyLock<crate::lifecycle::KeyedLocks> =
    std::sync::LazyLock::new(Default::default);

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

pub fn device_router(hub: &'static crate::detect::hub::DetectHub) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/list", get(list_devices))
        .route("/add", post(add_device))
        .route("/update/{id}", post(update_device))
        .route("/remove/{id}", post(remove_device))
        .with_state(hub)
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
    #[serde(default = "default_record")]
    record: bool,
    #[serde(default)]
    config: DeviceConfig,
}

fn default_record() -> bool {
    true
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
                // GB28181 streams are published by ZLM's RtpServer under the
                // `rtp` app, so they need a different FLV url than pipe streams.
                flv_url: if device.input_type == "gb28181" {
                    build_gb_flv_url(&device.id)
                } else {
                    build_flv_url(&device.id)
                },
                device,
            })
            .collect(),
    ))
}

async fn add_device(
    State(detect_hub): State<&'static crate::detect::hub::DetectHub>,
    Json(payload): Json<DevicePayload>,
) -> ApiJsonResult<DeviceInfo> {
    let conn = app_db_conn()?;
    let now = Utc::now();
    let name = payload.name.trim().to_string();
    let config = normalize_device_config(&payload.config)?;
    let device = DeviceInfo {
        id: payload.id.unwrap_or_else(|| device_id_from_name(&name)),
        name,
        input_type: payload.input_type.trim().to_string(),
        input_value: payload.input_value.trim().to_string(),
        description: payload.description.unwrap_or_default().trim().to_string(),
        include_audio: payload.include_audio,
        record: payload.record,
        config,
        created_at: now,
        updated_at: now,
    };
    validate_device(&device)?;
    crate::x11::validate_input(&device.input_type, &device.input_value).await?;
    let _operation = DEVICE_OPERATIONS.lock(&device.id).await;
    if nvr_db::device::get(&device.id, &conn).await?.is_some() {
        return Err(anyhow::anyhow!("device already exists").into());
    }
    save_and_apply_device(detect_hub, &device, None, &conn).await?;
    Ok(ok_json(device))
}

async fn update_device(
    State(detect_hub): State<&'static crate::detect::hub::DetectHub>,
    Path(id): Path<String>,
    Json(payload): Json<DevicePayload>,
) -> ApiJsonResult<DeviceInfo> {
    let _operation = DEVICE_OPERATIONS.lock(&id).await;
    let conn = app_db_conn()?;
    let existing = nvr_db::device::get(&id, &conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("device not found"))?;
    let config = normalize_device_config(&payload.config)?;
    let device = DeviceInfo {
        id,
        name: payload.name.trim().to_string(),
        input_type: payload.input_type.trim().to_string(),
        input_value: payload.input_value.trim().to_string(),
        description: payload.description.unwrap_or_default().trim().to_string(),
        include_audio: payload.include_audio,
        record: payload.record,
        config,
        created_at: existing.created_at,
        updated_at: Utc::now(),
    };
    validate_device(&device)?;
    crate::x11::validate_input(&device.input_type, &device.input_value).await?;
    save_and_apply_device(detect_hub, &device, Some(&existing), &conn).await?;
    Ok(ok_json(device))
}

async fn remove_device(
    State(detect_hub): State<&'static crate::detect::hub::DetectHub>,
    Path(id): Path<String>,
) -> ApiJsonResult<String> {
    let _operation = DEVICE_OPERATIONS.lock(&id).await;
    let conn = app_db_conn()?;
    nvr_db::device::delete(&id, &conn).await?;
    manager::remove_pipe(&id).await?;
    // The tap holds a video subscription to a pipe that no longer exists; it
    // would end on its own at EOF, but stop it now so the slot frees promptly.
    detect_hub.stop(&id);
    if let Some(bridge) = crate::gb::bridge() {
        bridge.unregister_mapping(&id).await;
    }
    // Idempotent no-op for non-onvif devices; drops the onvif registry entry
    // otherwise so PTZ / re-resolve don't keep a stale config for a gone device.
    crate::onvif::remove(&id);
    Ok(ok_json("success".to_string()))
}

async fn save_and_apply_device(
    detect_hub: &'static crate::detect::hub::DetectHub,
    device: &DeviceInfo,
    existing: Option<&DeviceInfo>,
    conn: &turso::Connection,
) -> anyhow::Result<()> {
    // Validation has already completed. Persist before reconciliation so an
    // auto-start retry can read the new detection settings.
    nvr_db::device::upsert(device, conn).await?;
    let applied: anyhow::Result<()> = async {
        // On an input_type change involving gb28181, clean up the old kind's
        // resources first: leaving gb28181 must drop the stale pull mapping (+ any
        // active pull), and entering gb28181 must remove the old pipe (the gb arm
        // builds none, so `ensure_device_pipe` won't replace it). Both are
        // idempotent no-ops otherwise; non-gb↔non-gb keeps its upsert-in-place path.
        if let Some(existing) = existing.filter(|old| old.input_type != device.input_type) {
            if existing.input_type == "gb28181" {
                if let Some(bridge) = crate::gb::bridge() {
                    bridge.unregister_mapping(&device.id).await;
                }
            }
            // Leaving onvif must drop the registry entry (PTZ / stream re-resolve
            // read from it), mirroring the gb28181 mapping cleanup above. The
            // supervisor task itself is stopped by the upsert that replaces it.
            if existing.input_type == "onvif" {
                crate::onvif::remove(&device.id);
            }
            if device.input_type == "gb28181" {
                manager::remove_pipe(&device.id).await?;
            }
        }
        ensure_device_pipe(detect_hub, device).await?;
        Ok(())
    }
    .await;
    if let Err(error) = applied {
        let restored: anyhow::Result<()> = async {
            match existing {
                Some(old) => {
                    nvr_db::device::upsert(old, conn).await?;
                    ensure_device_pipe(detect_hub, old).await?;
                }
                None => {
                    nvr_db::device::delete(&device.id, conn).await?;
                }
            }
            Ok(())
        }
        .await;
        if let Err(restore_error) = restored {
            return Err(error.context(format!("device rollback also failed: {restore_error:#}")));
        }
        return Err(error);
    }
    Ok(())
}

fn validate_device(device: &DeviceInfo) -> anyhow::Result<()> {
    if device.id.trim().is_empty() {
        anyhow::bail!("device id is required");
    }
    if device.name.is_empty() {
        return Err(anyhow::anyhow!("device name is required"));
    }
    if device.input_type.is_empty() {
        return Err(anyhow::anyhow!("input type is required"));
    }
    if device.input_value.is_empty() {
        return Err(anyhow::anyhow!("input value is required"));
    }
    crate::detect::control::validate_detect_config(device.config.detect.as_ref())?;
    crate::init::device::validate_device_input(device)
}

fn normalize_device_config(config: &DeviceConfig) -> anyhow::Result<DeviceConfig> {
    Ok(DeviceConfig {
        detect: crate::detect::control::normalize_detect_config(config.detect.as_ref())?,
    })
}

#[cfg(test)]
#[path = "device_test.rs"]
mod device_test;
