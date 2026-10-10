use std::sync::Arc;

use anyhow::Context;

use nvr_db::device::DeviceInfo;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::{db::app_db_conn, manager};
use media_pipe_core::{EncodeConfig, InputConfig, PipeConfig};

pub(crate) fn init_device_pipes(
    detect_hub: &'static crate::detect::hub::DetectHub,
    zlm_ready: oneshot::Receiver<()>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    tokio::spawn(async move {
        tokio::select! {
         _ = zlm_ready => {
            log::info!("ZLM server is ready");
            init_device_pipes_inner(detect_hub).await.unwrap_or_else(|e| {
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

async fn init_device_pipes_inner(
    detect_hub: &'static crate::detect::hub::DetectHub,
) -> anyhow::Result<()> {
    let conn = app_db_conn()?;
    let devices = nvr_db::device::list(&conn).await?;
    let total = devices.len();

    for device in devices {
        if let Err(err) = ensure_device_pipe(detect_hub, &device).await {
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

#[derive(serde::Deserialize)]
struct GbInput {
    device_id: String,
    channel_id: String,
}

/// Validate source configuration before any persistence or runtime mutation.
/// Offline cameras remain valid; this checks shape, not network reachability.
pub(crate) fn validate_device_input(device: &DeviceInfo) -> anyhow::Result<()> {
    match device.input_type.as_str() {
        "gb28181" => {
            let cfg: GbInput = serde_json::from_str(&device.input_value)
                .context("invalid gb28181 device config")?;
            if cfg.device_id.trim().is_empty() || cfg.channel_id.trim().is_empty() {
                anyhow::bail!("gb28181 device_id and channel_id are required");
            }
        }
        "onvif" => {
            let cfg: nvr_onvif::OnvifConfig =
                serde_json::from_str(&device.input_value).context("invalid onvif device config")?;
            if cfg.host.trim().is_empty() || cfg.port == 0 {
                anyhow::bail!("onvif host and a nonzero port are required");
            }
        }
        "xiaomi" => {
            let cfg: crate::xiaomi::XiaomiConfig = serde_json::from_str(&device.input_value)
                .context("invalid xiaomi device config")?;
            if [&cfg.user_id, &cfg.token, &cfg.did, &cfg.model, &cfg.ip]
                .iter()
                .any(|value| value.trim().is_empty())
            {
                anyhow::bail!("xiaomi account and device fields are required");
            }
        }
        "net" | "rtsp" | "rtmp" | "stream" => {
            reqwest::Url::parse(&device.input_value).context("invalid input URL")?;
        }
        "file" | "v4l2" | "x11grab" | "lavfi" => {}
        _ => anyhow::bail!("unsupported input type: {}", device.input_type),
    }
    Ok(())
}

pub(crate) async fn ensure_device_pipe(
    detect_hub: &'static crate::detect::hub::DetectHub,
    device: &DeviceInfo,
) -> anyhow::Result<()> {
    validate_device_input(device)?;
    crate::x11::validate_input(&device.input_type, &device.input_value).await?;
    // Xiaomi cameras bypass ffmpeg entirely: a native worker pushes the
    // decoded H264 straight into a ZLM Media. `input_value` carries the
    // XiaomiConfig as JSON.
    if device.input_type == "xiaomi" {
        let cfg: crate::xiaomi::XiaomiConfig =
            serde_json::from_str(&device.input_value).context("invalid xiaomi device config")?;
        let media = Arc::new(rszlm::media::Media::new_with_default_vhost(
            DEVICE_APP,
            device.id.as_str(),
            0.0,
            device.record,
            false,
        ));
        manager::upsert_xiaomi(&device.id, media, cfg, true).await?;
        crate::detect::control::reconcile_detection(detect_hub, device).await;
        return Ok(());
    }

    // GB28181 cameras have no always-on pipe: they only register a mapping so
    // the on-demand bridge can INVITE-pull when a viewer opens the stream. The
    // `input_value` carries `{ "device_id": "...", "channel_id": "..." }`.
    if device.input_type == "gb28181" {
        let gb: GbInput =
            serde_json::from_str(&device.input_value).context("invalid gb28181 device config")?;
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
        crate::detect::control::reconcile_detection(detect_hub, device).await;
        return Ok(());
    }

    // ONVIF cameras: the RTSP URL isn't stored — it's resolved from the
    // camera's media service, and can move on a reboot/config change. So the
    // `input_value` carries the OnvifConfig; we register it (for PTZ / the REST
    // surface) and spawn a supervisor that resolves the RTSP URI just-in-time
    // and re-resolves on every reconnect, feeding the shared RTSP -> ZLM pipe.
    if device.input_type == "onvif" {
        let cfg: nvr_onvif::OnvifConfig =
            serde_json::from_str(&device.input_value).context("invalid onvif device config")?;
        crate::onvif::register(&device.id, cfg.clone());
        let media = Arc::new(rszlm::media::Media::new_with_default_vhost(
            DEVICE_APP,
            device.id.as_str(),
            0.0,
            device.record,
            false,
        ));
        manager::upsert_onvif(&device.id, media, cfg, device.include_audio, true).await?;
        crate::detect::control::reconcile_detection(detect_hub, device).await;
        return Ok(());
    }

    // Platform live streams (Douyin/Bilibili/Twitch… room pages): there is no
    // stable URL to hand to ffmpeg — the pull address is temporary and signed.
    // `input_value` stores the room/page URL; a supervisor worker resolves it
    // via yt-dlp right before opening and again on every reconnect.
    if device.input_type == "stream" {
        let media = Arc::new(rszlm::media::Media::new_with_default_vhost(
            DEVICE_APP,
            device.id.as_str(),
            0.0,
            device.record,
            false,
        ));
        manager::upsert_stream(
            &device.id,
            media,
            device.input_value.clone(),
            device.include_audio,
            true,
        )
        .await?;
        crate::detect::control::reconcile_detection(detect_hub, device).await;
        return Ok(());
    }

    if matches!(device.input_type.as_str(), "net" | "rtsp" | "rtmp") {
        return manager::upsert_network_device(device, detect_hub).await;
    }

    let input = match device.input_type.as_str() {
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
        DEVICE_APP,
        device.id.as_str(),
        0.0,
        device.record,
        false,
    ));
    let mut outputs = media_pipe_zlm::zlm_outputs(media, device.include_audio);
    // Capture devices produce raw frames, which ZLM cannot distribute directly.
    if matches!(device.input_type.as_str(), "v4l2" | "x11grab" | "lavfi") {
        outputs[0].encode = Some(EncodeConfig {
            pixel_format: Some("yuv420p".to_string()),
            ..EncodeConfig::default()
        });
    }

    let config = PipeConfig { input, outputs };
    manager::update_pipe(&device.id, config).await?;
    crate::detect::control::reconcile_detection(detect_hub, device).await;
    Ok(())
}

/// Playable HTTP-FLV URL as a same-origin path through the `/media` reverse
/// proxy (see `proxy.rs`), not ZLM's direct `127.0.0.1:8553`. A relative path
/// keeps playback working behind port-forwarding / remote access, where only
/// the API port is reachable and ZLM's port is not.
/// ZLM app name under which regular device streams are published. Kept distinct
/// from the compositor (`switcher`), audio-mixer (`mixer`), and GB28181 (`rtp`)
/// apps so the four stream families never collide.
pub(crate) const DEVICE_APP: &str = "device";

pub(crate) fn build_flv_url(device_id: &str) -> String {
    format!("/media/{}/{}.live.flv", DEVICE_APP, device_id)
}

/// GB28181 streams are published by ZLM's RtpServer under the `rtp` app (not
/// `live`), so their playable URL differs from `build_flv_url`. Same `/media`
/// proxy path (see `build_flv_url`).
pub(crate) fn build_gb_flv_url(device_id: &str) -> String {
    format!("/media/rtp/{}.live.flv", device_id)
}
