use std::sync::Arc;

use nvr_db::device::DeviceInfo;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::{db::app_db_conn, manager};
use media_pipe_core::{InputConfig, PipeConfig};

pub(crate) fn init_device_pipes(
    zlm_ready: oneshot::Receiver<()>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    tokio::spawn(async move {
        tokio::select! {
         _ = zlm_ready => {
            log::info!("ZLM server is ready");
            init_device_pipes_inner().await.unwrap_or_else(|e| {
                log::error!("Failed to init device pipes: {:#}", e);
             });
         },
         _ = cancel.cancelled() => {
             log::info!("Cancel signal received");
             return;
         },
        }
    });
    Ok(())
}

async fn init_device_pipes_inner() -> anyhow::Result<()> {
    let conn = app_db_conn()?;
    let devices = nvr_db::device::list(&conn).await?;
    let total = devices.len();

    for device in devices {
        if let Err(err) = ensure_device_pipe(&device).await {
            log::error!("Failed to init device pipe {}: {:#}", device.id, err);
        } else {
            log::info!("Initialized device pipe {}", device.id);
        }
    }

    log::info!("Device pipe initialization finished, total={}", total);

    // Restore persisted compositor programs now that device streams are being
    // published to ZLM. Spawned so its grace period / retries don't block.
    tokio::spawn(async {
        crate::compositor::restore_all().await;
    });

    // Same for the audio mixer buses (they pull the same device streams).
    tokio::spawn(async {
        crate::audiomixer::restore_all().await;
    });

    Ok(())
}

pub(crate) async fn ensure_device_pipe(device: &DeviceInfo) -> anyhow::Result<()> {
    // Xiaomi cameras bypass ffmpeg entirely: a native worker pushes the
    // decoded H264 straight into a ZLM Media. `input_value` carries the
    // XiaomiConfig as JSON.
    if device.input_type == "xiaomi" {
        let cfg: crate::xiaomi::XiaomiConfig = serde_json::from_str(&device.input_value)
            .map_err(|e| anyhow::anyhow!("invalid xiaomi device config: {e}"))?;
        let media = Arc::new(rszlm::media::Media::new_with_default_vhost(
            "live",
            device.id.as_str(),
            0.0,
            device.record,
            false,
        ));
        return manager::upsert_xiaomi(&device.id, media, cfg, true).await;
    }

    // GB28181 cameras have no always-on pipe: they only register a mapping so
    // the on-demand bridge can INVITE-pull when a viewer opens the stream. The
    // `input_value` carries `{ "device_id": "...", "channel_id": "..." }`.
    if device.input_type == "gb28181" {
        #[derive(serde::Deserialize)]
        struct GbInput {
            device_id: String,
            channel_id: String,
        }
        let gb: GbInput = serde_json::from_str(&device.input_value)
            .map_err(|e| anyhow::anyhow!("invalid gb28181 device config: {e}"))?;
        match crate::gb::bridge() {
            Some(bridge) => {
                // stream id == nvr device id (the ZLM stream name we pull into).
                bridge.register_mapping(
                    &device.id,
                    &gb.device_id,
                    &gb.channel_id,
                    gb28181::Transport::Udp,
                );
                log::info!(
                    "gb28181: registered mapping {} -> {}/{}",
                    device.id,
                    gb.device_id,
                    gb.channel_id
                );
            }
            None => {
                log::warn!(
                    "gb28181 device {} added but GB support is not active \
                     (NVR_GB_ENABLE!=1, or the platform failed to bind — see startup logs)",
                    device.id
                );
            }
        }
        return Ok(());
    }

    let input = match device.input_type.as_str() {
        "net" | "rtsp" | "rtmp" => InputConfig::Network {
            url: device.input_value.clone(),
        },
        "file" => InputConfig::File {
            path: device.input_value.clone(),
        },
        "v4l2" | "x11grab" | "lavfi" => InputConfig::Device {
            display: device.input_value.clone(),
            format: device.input_type.clone(),
        },
        _ => {
            return Err(anyhow::anyhow!(
                "unsupported input type: {}",
                device.input_type
            ));
        }
    };

    // hls_enabled drives recording: ZLM only produces the HLS segments that
    // get archived (on_record_ts) when this is on. Live view uses FLV, which
    // is independent, so disabling HLS just turns recording off.
    let media = Arc::new(rszlm::media::Media::new_with_default_vhost(
        "live",
        device.id.as_str(),
        0.0,
        device.record,
        false,
    ));
    let outputs = media_pipe_zlm::zlm_outputs(media, device.include_audio);

    let config = PipeConfig { input, outputs };
    manager::update_pipe(&device.id, config).await
}

/// Playable HTTP-FLV URL as a same-origin path through the `/media` reverse
/// proxy (see `proxy.rs`), not ZLM's direct `127.0.0.1:8553`. A relative path
/// keeps playback working behind port-forwarding / remote access, where only
/// the API port is reachable and ZLM's port is not.
pub(crate) fn build_flv_url(device_id: &str) -> String {
    format!("/media/live/{}.live.flv", device_id)
}

/// GB28181 streams are published by ZLM's RtpServer under the `rtp` app (not
/// `live`), so their playable URL differs from `build_flv_url`. Same `/media`
/// proxy path (see `build_flv_url`).
pub(crate) fn build_gb_flv_url(device_id: &str) -> String {
    format!("/media/rtp/{}.live.flv", device_id)
}
