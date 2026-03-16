use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use tokio_linux_video::Device;

use crate::handler::ApiJsonResult;

pub fn system_router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/list/device/foramts", get(list_device_foramts))
        .route("/list/v4l2/devices", get(list_v4l2_device))
        .route("/list/x11grab/devices", get(list_x11grab_device))
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

async fn list_device_foramts(
    Json(request): Json<DeviceListRequest>,
) -> ApiJsonResult<Vec<DeviceFormatInfoItem>> {
    match request.kind {
        0 => {
            let video_devices = match request.direction {
                0 => ffmpeg_bus::device::input_video_format_list(),
                1 => ffmpeg_bus::device::output_video_format_list(),
                _ => return Err(anyhow::anyhow!("invalid direction").into()),
            }?;

            Ok(Json(
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

            Ok(Json(
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
        Ok(Json(device_names))
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
        Ok(Json(list))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(anyhow::anyhow!("not supported").into())
    }
}
