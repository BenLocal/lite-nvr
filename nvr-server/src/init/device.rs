use std::sync::Arc;

use nvr_db::device::DeviceInfo;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::{
    db::app_db_conn,
    manager,
    media::types::{InputConfig, OutputConfig, OutputDest, PipeConfig},
};

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
    Ok(())
}

pub(crate) async fn ensure_device_pipe(device: &DeviceInfo) -> anyhow::Result<()> {
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

    #[cfg(feature = "zlm")]
    let output = OutputConfig::new(
        OutputDest::Zlm(Arc::new(rszlm::media::Media::new_with_default_vhost(
            "live",
            device.id.as_str(),
            0.0,
            true,
            false,
        ))),
        None,
    );

    #[cfg(not(feature = "zlm"))]
    let output = {
        return Err(anyhow::anyhow!(
            "zlm feature is disabled, device auto-pipeline is unavailable"
        ));
    };

    let config = PipeConfig {
        input,
        outputs: vec![output],
    };
    manager::update_pipe(&device.id, config).await
}

pub(crate) fn build_flv_url(device_id: &str) -> String {
    format!("http://127.0.0.1:8553/live/{}.live.flv", device_id)
}
