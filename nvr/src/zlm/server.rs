use rszlm::{
    init::{EnvIni, EnvInitBuilder},
    server::{http_server_start, rtmp_server_start, rtsp_server_start},
};
use std::path::{Path, PathBuf};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use crate::zlm::cmd::{handler_zlm_cmd, init_zlm_cmd_sender};

pub(crate) fn start_zlm_server(
    cancel: CancellationToken,
    ready_tx: oneshot::Sender<()>,
) -> anyhow::Result<()> {
    let mut rx = init_zlm_cmd_sender()?;
    tokio::spawn(async move {
        let cancel_clone = cancel.clone();
        let runtime = tokio::runtime::Handle::current();
        let handle = tokio::task::spawn_blocking(move || {
            EnvInitBuilder::default()
                .log_level(0)
                .log_mask(0)
                .thread_num(20)
                .build();

            {
                let ini = EnvIni::global().lock().unwrap();
                ini.set_option("hls.broadcastRecordTs", "1");
                ini.set_option("hls.segDur", "60");
            }

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
                let runtime_clone = runtime.clone();
                events.on_record_ts(move |record| {
                    let record_app = record.ts.app();
                    let record_stream = record.ts.stream();
                    let record_path = record.ts.file_path();
                    let record_start_time = record.ts.start_time();
                    let record_duration = record.ts.duration();
                    let record_file_name = record.ts.file_name();
                    let record_folder = record.ts.folder();
                    let record_vhost = record.ts.vhost();
                    let record_file_size = record.ts.file_size();
                    let runtime_inner = runtime_clone.clone();
                    runtime_inner.spawn(async move {
                        if let Err(err) = persist_record_ts(
                            record_start_time,
                            record_duration,
                            record_file_size,
                            record_file_name,
                            record_path,
                            record_folder,
                            record_app,
                            record_stream,
                            record_vhost,
                        )
                        .await
                        {
                            log::error!("ZLM: persist record ts failed: {:#}", err);
                        }
                    });
                });
            }

            let _ = ready_tx.send(());

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

fn record_archive_root() -> anyhow::Result<PathBuf> {
    Ok(std::env::current_dir()?.join("data").join("records"))
}

async fn archive_record_file(stream: &str, file_name: &str, source_path: &str) -> anyhow::Result<PathBuf> {
    let relative_path = Path::new(file_name);
    let target_path = record_archive_root()?.join(stream).join(relative_path);
    let parent = target_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid archive target path"))?;
    tokio::fs::create_dir_all(parent).await?;
    tokio::fs::copy(source_path, &target_path).await?;
    Ok(target_path)
}

async fn persist_record_ts(
    start_time: u64,
    duration: f32,
    file_size: usize,
    file_name: String,
    file_path: String,
    _folder: String,
    app: String,
    stream: String,
    vhost: String,
) -> anyhow::Result<()> {
    let conn = crate::db::app_db_conn()?;
    let now = chrono::Utc::now();
    let archived_path = archive_record_file(&stream, &file_name, &file_path).await?;
    let archived_path_string = archived_path.to_string_lossy().to_string();
    let archived_folder = archived_path
        .parent()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_default();
    let archived_size = tokio::fs::metadata(&archived_path)
        .await?
        .len() as usize;
    let meta = ffmpeg_bus::metadata::probe(&archived_path_string)?;
    let video_stream = meta
        .streams
        .iter()
        .find(|stream| stream.codec_type == "video");
    let audio_stream = meta
        .streams
        .iter()
        .find(|stream| stream.codec_type == "audio");
    let record = nvr_db::record_segment::RecordSegment {
        id: uuid::Uuid::new_v4().simple().to_string(),
        record_type: 0,
        start_time,
        duration,
        file_size: archived_size.max(file_size),
        file_name,
        file_path: archived_path_string,
        folder: archived_folder,
        app,
        stream,
        vhost,
        video_codec: video_stream
            .map(|stream| stream.codec_name.clone())
            .unwrap_or_default(),
        video_width: video_stream
            .and_then(|stream| stream.width)
            .unwrap_or_default() as i32,
        video_height: video_stream
            .and_then(|stream| stream.height)
            .unwrap_or_default() as i32,
        video_fps: video_stream
            .and_then(|stream| parse_rate(&stream.rate))
            .unwrap_or_default(),
        video_bit_rate: meta.format.bit_rate,
        audio_codec: audio_stream
            .map(|stream| stream.codec_name.clone())
            .unwrap_or_default(),
        audio_sample_rate: audio_stream
            .and_then(|stream| stream.sample_rate)
            .unwrap_or_default() as i32,
        audio_channels: audio_stream
            .and_then(|stream| stream.channels)
            .unwrap_or_default() as i32,
        audio_bit_rate: 0,
        reserve_text1: String::new(),
        reserve_text2: String::new(),
        reserve_text3: String::new(),
        reserve_int1: 0,
        reserve_int2: 0,
        create_time: now,
        update_time: now,
    };
    nvr_db::record_segment::upsert(&record, &conn).await
}

fn parse_rate(value: &str) -> Option<f32> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.parse::<f32>().ok()?;
    let denominator = denominator.parse::<f32>().ok()?;
    if denominator == 0.0 {
        return None;
    }
    Some(numerator / denominator)
}
