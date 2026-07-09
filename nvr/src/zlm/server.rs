use rszlm::{
    init::{EnvIni, EnvInitBuilder},
    server::{http_server_start, rtmp_server_start, rtsp_server_start},
};
use std::path::{Path, PathBuf};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

/// ZLM invokers wrap a C++ `shared_ptr` and are documented callable from any
/// thread; the rszlm binding just holds a raw pointer, so it isn't auto-Send.
struct SendPublishInvoker(rszlm::event::PublishAuthInvoker);
unsafe impl Send for SendPublishInvoker {}

impl SendPublishInvoker {
    /// Whole-struct receiver so async blocks capture the Send wrapper, not the
    /// non-Send inner field (Rust 2021 captures fields individually).
    fn call(&self, err_msg: &str, enable_mp4: bool, enable_hls: bool) -> anyhow::Result<()> {
        self.0.call(err_msg, enable_mp4, enable_hls)
    }
}

/// Stop every ZLM listener and kill its sessions NOW, on a fully-alive
/// process. Leaving that to exit-time C++ static destruction meant live
/// sessions (e.g. an external RTSP pusher) were torn down while the statics
/// they touch were half-destroyed — the recurring shutdown segfault. Call
/// after all media producers are stopped, right before process exit.
pub(crate) fn stop_all() {
    rszlm::server::stop_all_server();
}

pub(crate) fn start_zlm_server(
    cancel: CancellationToken,
    ready_tx: oneshot::Sender<()>,
) -> anyhow::Result<()> {
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
                let runtime_pub = runtime.clone();
                events.on_media_publish(move |media| {
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
                    // Authorize the publish (empty err = OK). Required for external
                    // pushes such as a GB28181 pull's RtpProcess — without it ZLM
                    // buffers frames waiting on this hook, then times out and
                    // detaches, so the stream never goes live.
                    //
                    // MUST NOT run on this thread: the hook fires on the session's
                    // own event-poller thread, and ZLM's invoker `async()`s the
                    // ANNOUNCE continuation back to that same poller — which
                    // executes it INLINE, right above these Rust frames. When the
                    // continuation throws (e.g. pushing a stream name that already
                    // exists → "already publishing" shutdown), the C++ exception
                    // unwinds through Rust and aborts the whole process ("Rust
                    // cannot catch foreign exceptions"). Invoking from a tokio
                    // thread makes ZLM post the continuation instead, so any throw
                    // stays on a pure C++ stack and is handled by ZLM itself.
                    // The hook's invoker pointer is only valid during the
                    // callback; clone (mk_publish_auth_invoker_clone) takes an
                    // owned copy that stays valid for the deferred call.
                    let invoker = SendPublishInvoker(media.auth_invoker.clone());
                    runtime_pub.spawn(async move {
                        if let Err(e) = invoker.call("", false, false) {
                            log::warn!("ZLM: publish auth invoke failed: {e:#}");
                        }
                    });
                });
                let runtime_nf = runtime.clone();
                events.on_media_not_found(move |media| {
                    let app = media.url_info.app();
                    let stream = media.url_info.stream();
                    log::info!("ZLM: media not found, app: {app}, stream: {stream}");
                    let Some(bridge) = crate::gb::bridge() else {
                        return true; // GB disabled: nothing to provide, but don't error
                    };
                    let runtime_inner = runtime_nf.clone();
                    // The hook is synchronous; do the async pull on the runtime
                    // and return true so ZLM keeps the request open.
                    runtime_inner.spawn(async move {
                        bridge.handle_media_not_found(&stream).await;
                    });
                    true
                });
                let runtime_nr = runtime.clone();
                events.on_media_no_reader(move |media| {
                    let stream = media.sender.stream();
                    let Some(bridge) = crate::gb::bridge() else {
                        return;
                    };
                    let runtime_inner = runtime_nr.clone();
                    runtime_inner.spawn(async move {
                        bridge.handle_media_no_reader(&stream).await;
                    });
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
        cancel.cancelled().await;
        log::info!("ZLM: server cancelled");

        let _ = handle.await;
        log::info!("ZLM: server finished");
    });

    Ok(())
}

fn record_archive_root() -> anyhow::Result<PathBuf> {
    Ok(crate::config::config().record_dir())
}

async fn archive_record_file(
    stream: &str,
    file_name: &str,
    source_path: &str,
) -> anyhow::Result<PathBuf> {
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
    let archived_size = tokio::fs::metadata(&archived_path).await?.len() as usize;
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
