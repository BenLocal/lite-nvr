use axum::{Json, Router, routing::get};
use serde::{Deserialize, Serialize};

use crate::handler::ApiJsonResult;

pub fn system_router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/list/devices", get(list_devices))
}

#[derive(Serialize, Deserialize)]
struct DeviceListRequest {
    /// 0: video, 1: audio
    kind: u32,
    /// 0: input, 1: output
    direction: u32,
}

#[derive(Serialize, Deserialize)]
struct DeviceInfoItem {
    name: String,
    description: String,
}

async fn index() -> &'static str {
    "system route!"
}

async fn list_devices(
    Json(request): Json<DeviceListRequest>,
) -> ApiJsonResult<Vec<DeviceInfoItem>> {
    match request.kind {
        0 => {
            let video_devices = match request.direction {
                0 => ffmpeg_bus::device::input_video_list(),
                1 => ffmpeg_bus::device::output_video_list(),
                _ => return Err(anyhow::anyhow!("invalid direction").into()),
            }?;

            Ok(Json(
                video_devices
                    .iter()
                    .map(|d| DeviceInfoItem {
                        name: d.name().to_string(),
                        description: d.description().to_string(),
                    })
                    .collect::<Vec<DeviceInfoItem>>(),
            ))
        }
        1 => {
            let audio_devices = match request.direction {
                0 => ffmpeg_bus::device::input_audio_list(),
                1 => ffmpeg_bus::device::output_audio_list(),
                _ => return Err(anyhow::anyhow!("invalid direction").into()),
            }?;

            Ok(Json(
                audio_devices
                    .iter()
                    .map(|d| DeviceInfoItem {
                        name: d.name().to_string(),
                        description: d.description().to_string(),
                    })
                    .collect::<Vec<DeviceInfoItem>>(),
            ))
        }
        _ => Err(anyhow::anyhow!("invalid kind").into()),
    }
}
