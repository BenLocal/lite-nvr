#![cfg(feature = "zlm")]

use rszlm::{
    event,
    init::EnvInitBuilder,
    server::{http_server_start, rtmp_server_start, rtsp_server_start},
};
use tokio_util::sync::CancellationToken;

pub(crate) fn start_zlm_server(cancel: CancellationToken) {
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
                let mut events = event::EVENTS.write().unwrap();
                events.on_media_publish(|_media| {
                    log::info!("ZLM: media publish");
                });
            }

            loop {
                if cancel_clone.is_cancelled() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
        tokio::select! {
            _ = handle => {
                log::info!("ZLM: server finished");
            }
            _ = cancel.cancelled() => {
                log::info!("ZLM: server cancelled");
            }
        }
    });
}
