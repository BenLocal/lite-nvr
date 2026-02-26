use rszlm::{
    init::EnvInitBuilder,
    server::{http_server_start, rtmp_server_start, rtsp_server_start},
};
use tokio_util::sync::CancellationToken;

use crate::zlm::cmd::{handler_zlm_cmd, init_zlm_cmd_sender};

pub(crate) fn start_zlm_server(cancel: CancellationToken) -> anyhow::Result<()> {
    let mut rx = init_zlm_cmd_sender()?;
    tokio::spawn(async move {
        let cancel_clone = cancel.clone();
        let handle = tokio::task::spawn_blocking(move || {
            EnvInitBuilder::default()
                .log_level(0)
                .log_mask(0)
                .thread_num(20)
                .build();

            http_server_start(8553, false);
            rtsp_server_start(8554, false);
            rtmp_server_start(8555, false);

            {
                let mut events = rszlm::event::EVENTS.write().unwrap();
                events.on_media_publish(|media| {
                    let url_info = format!(
                        "{}://{}:{}/{}/{}/{}",
                        media.url_info.schema(),
                        media.url_info.host(),
                        media.url_info.port(),
                        media.url_info.vhost(),
                        media.url_info.app(),
                        media.url_info.stream()
                    );
                    log::info!("ZLM: media publish, url: {}", url_info);
                });
                events.on_media_not_found(|media| {
                    let url_info = format!(
                        "{}://{}:{}/{}/{}/{}",
                        media.url_info.schema(),
                        media.url_info.host(),
                        media.url_info.port(),
                        media.url_info.vhost(),
                        media.url_info.app(),
                        media.url_info.stream()
                    );
                    log::info!("ZLM: media not found, url: {}", url_info);
                    true
                });
                events.on_record_ts(|record| {
                    let record_app = record.ts.app();
                    let record_stream = record.ts.stream();
                    let record_path = record.ts.file_path();
                    let record_start_time = record.ts.start_time();
                    let record_duration = record.ts.duration();
                    log::info!("ZLM: record ts, app: {}, stream: {}, path: {}, start_time: {}, duration: {}", record_app, record_stream, record_path, record_start_time, record_duration);
                });
            }

            loop {
                if cancel_clone.is_cancelled() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    log::info!("ZLM: server cancelled");
                    break;
                }
                Some(cmd) = rx.recv() => {
                   if let Err(e) = handler_zlm_cmd(cmd) {
                        log::error!("ZLM: handler_zlm_cmd error: {:?}", e);
                   }
                }
            }
        }

        let _ = handle.await;
        log::info!("ZLM: server finished");
    });

    Ok(())
}
