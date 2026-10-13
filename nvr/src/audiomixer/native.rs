//! Keep native-device original media separate from the processed output.
use media_pipe_core::{InputConfig, Pipe, PipeConfig};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub const SOURCE_APP: &str = "nvr-source";

pub struct NativeOutput {
    pub device_id: String,
    pub source_app: String,
    pub source_id: String,
    pub output_app: String,
    pub record: bool,
    pub include_original: bool,
    pub settings: Option<nvr_db::device::OutputConfig>,
}

pub fn required(id: &str, use_global: bool) -> bool {
    (super::output::config().enabled && use_global) || super::MIXER.has_source(id)
}

pub fn spawn(config: NativeOutput, cancel: CancellationToken) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if cancel.is_cancelled() {
                break;
            }
            if !crate::zlm::media_cache::MediaCache.is_live(&config.source_app, &config.source_id) {
                tokio::select! { _ = cancel.cancelled() => break, _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {} }
                continue;
            }
            let media = Arc::new(rszlm::media::Media::new_with_default_vhost(
                &config.output_app,
                &config.device_id,
                0.0,
                config.record,
                false,
            ));
            let outputs = match crate::device_output::outputs(
                &config.device_id,
                media,
                config.include_original,
                config.settings.as_ref(),
            )
            .await
            {
                Ok(outputs) => outputs,
                Err(e) => {
                    log::warn!("native program {}: {e:#}", config.device_id);
                    break;
                }
            };
            let port = match crate::zlm::server::rtsp_port() {
                Ok(p) => p,
                Err(e) => {
                    log::error!("native source port: {e:#}");
                    break;
                }
            };
            let pipe = Arc::new(Pipe::new(PipeConfig {
                input: InputConfig::Network {
                    url: format!(
                        "rtsp://127.0.0.1:{port}/{}/{}",
                        config.source_app, config.source_id
                    ),
                },
                outputs,
            }));
            super::output::register_pipe(&config.device_id, &pipe);
            let session = pipe.clone();
            let mut task = tokio::spawn(async move {
                session
                    .start(Some(std::collections::HashMap::from([(
                        "rtsp_transport".to_string(),
                        "tcp".to_string(),
                    )])))
                    .await;
            });
            tokio::select! { _ = &mut task => {}, _ = cancel.cancelled() => { pipe.cancel(); let _ = task.await; } }
            drop(pipe);
            tokio::select! { _ = cancel.cancelled() => break, _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {} }
        }
    })
}
