use axum::{Json, Router, http::StatusCode, routing::get};
use serde::{Deserialize, Serialize};

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

async fn index() -> &'static str {
    "system route!"
}

async fn list_devices(
    Json(request): Json<DeviceListRequest>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    match request.kind {
        0 => {
            let video_devices = match request.direction {
                0 => ffmpeg_bus::device::input_video_list(),
                1 => ffmpeg_bus::device::output_video_list(),
                _ => return Err((StatusCode::BAD_REQUEST, "invalid direction".to_string())),
            }
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            Ok(Json(video_devices.iter().map(|d| d.to_string()).collect()))
        }
        1 => {
            let audio_devices = match request.direction {
                0 => ffmpeg_bus::device::input_audio_list(),
                1 => ffmpeg_bus::device::output_audio_list(),
                _ => return Err((StatusCode::BAD_REQUEST, "invalid direction".to_string())),
            }
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            Ok(Json(audio_devices.iter().map(|d| d.to_string()).collect()))
        }
        _ => Err((StatusCode::BAD_REQUEST, "invalid kind".to_string())),
    }
}
