use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use tokio_linux_video::Device;

use crate::db::app_db_conn;
use crate::handler::{ApiJsonResult, ok_json};
use crate::init::device::build_flv_url;
use crate::manager;

pub fn system_router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/overview", get(overview))
        .route("/list/device/formats", get(list_device_formats))
        .route("/list/v4l2/devices", get(list_v4l2_device))
        .route("/list/x11grab/devices", get(list_x11grab_device))
}

#[derive(Serialize)]
struct OverviewResponse {
    device_total: usize,
    device_online: usize,
    device_offline: usize,
    record_segment_count: usize,
    record_total_bytes: u64,
    devices: Vec<OverviewDevice>,
}

#[derive(Serialize)]
struct OverviewDevice {
    id: String,
    name: String,
    input_type: String,
    description: String,
    online: bool,
    record: bool,
    flv_url: String,
}

/// System overview: device online/offline counts, recording storage totals, and
/// per-device live status (online = a pipe/worker is running for it).
async fn overview() -> ApiJsonResult<OverviewResponse> {
    let conn = app_db_conn()?;
    let devices = nvr_db::device::list(&conn).await?;

    let mut items = Vec::with_capacity(devices.len());
    let mut online = 0usize;
    for d in &devices {
        let is_online = manager::status(&d.id).await.unwrap_or(false);
        if is_online {
            online += 1;
        }
        items.push(OverviewDevice {
            id: d.id.clone(),
            name: d.name.clone(),
            input_type: d.input_type.clone(),
            description: d.description.clone(),
            online: is_online,
            record: d.record,
            flv_url: build_flv_url(&d.id),
        });
    }

    let total = devices.len();
    let record_segment_count = nvr_db::record_segment::count(&conn).await?;
    let record_total_bytes = nvr_db::record_segment::total_size(&conn).await?;

    Ok(ok_json(OverviewResponse {
        device_total: total,
        device_online: online,
        device_offline: total - online,
        record_segment_count,
        record_total_bytes,
        devices: items,
    }))
}

#[derive(Serialize, Deserialize)]
struct DeviceListRequest {
    /// 0: video, 1: audio
    kind: u32,
    /// 0: input, 1: output
    direction: u32,
}

#[derive(Serialize, Deserialize)]
struct DeviceFormatInfoItem {
    format: String,
    description: String,
    mime_types: Vec<String>,
    extensions: Vec<String>,
}

async fn index() -> &'static str {
    "system route!"
}

async fn list_device_formats(
    Json(request): Json<DeviceListRequest>,
) -> ApiJsonResult<Vec<DeviceFormatInfoItem>> {
    match request.kind {
        0 => {
            let video_devices = match request.direction {
                0 => ffmpeg_bus::device::input_video_format_list(),
                1 => ffmpeg_bus::device::output_video_format_list(),
                _ => return Err(anyhow::anyhow!("invalid direction").into()),
            }?;

            Ok(ok_json(
                video_devices
                    .iter()
                    .map(|d| DeviceFormatInfoItem {
                        format: d.name().to_string(),
                        description: d.description().to_string(),
                        mime_types: d.mime_types().iter().map(|m| m.to_string()).collect(),
                        extensions: d.extensions().iter().map(|e| e.to_string()).collect(),
                    })
                    .collect::<Vec<DeviceFormatInfoItem>>(),
            ))
        }
        1 => {
            let audio_devices = match request.direction {
                0 => ffmpeg_bus::device::input_audio_format_list(),
                1 => ffmpeg_bus::device::output_audio_format_list(),
                _ => return Err(anyhow::anyhow!("invalid direction").into()),
            }?;

            Ok(ok_json(
                audio_devices
                    .iter()
                    .map(|d| DeviceFormatInfoItem {
                        format: d.name().to_string(),
                        description: d.description().to_string(),
                        mime_types: d.mime_types().iter().map(|m| m.to_string()).collect(),
                        extensions: d.extensions().iter().map(|e| e.to_string()).collect(),
                    })
                    .collect::<Vec<DeviceFormatInfoItem>>(),
            ))
        }
        _ => Err(anyhow::anyhow!("invalid kind").into()),
    }
}

async fn list_v4l2_device() -> ApiJsonResult<Vec<String>> {
    #[cfg(target_os = "linux")]
    {
        let mut devices = Device::list().await?;

        let mut device_names = Vec::new();
        while let Some(device) = devices.fetch_next().await? {
            device_names.push(device.display().to_string());
        }
        Ok(ok_json(device_names))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(anyhow::anyhow!("not supported").into())
    }
}

/// List x11grab `-i` options (X11 display strings). Uses DISPLAY env when set;
/// otherwise returns a default `:0` so callers have at least one option.
async fn list_x11grab_device() -> ApiJsonResult<Vec<String>> {
    #[cfg(target_os = "linux")]
    {
        let mut list = Vec::new();
        if let Ok(display) = std::env::var("DISPLAY") {
            let display = display.trim();
            if !display.is_empty() {
                list.push(display.to_string());
                if !display.contains('.') {
                    list.push(format!("{}.0", display));
                }
            }
        }
        if list.is_empty() {
            list.push(":0".to_string());
        }
        Ok(ok_json(list))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(anyhow::anyhow!("not supported").into())
    }
}
