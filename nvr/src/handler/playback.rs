use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

use crate::{
    db::app_db_conn,
    handler::{ApiJsonResult, ApiResult, ok_json},
};

const PLAYBACK_BYTERANGE_SEGMENT_SECONDS: f64 = 5.0;

#[derive(Debug, Clone, Copy)]
struct ByteRangeSegment {
    offset: usize,
    length: usize,
    duration: f32,
}

async fn segment_file_exists(path: &str) -> bool {
    tokio::fs::metadata(path).await.is_ok()
}

async fn filter_existing_records(
    records: Vec<nvr_db::record_segment::RecordSegment>,
) -> Vec<nvr_db::record_segment::RecordSegment> {
    let mut existing = Vec::with_capacity(records.len());
    for record in records {
        if segment_file_exists(&record.file_path).await {
            existing.push(record);
        } else {
            log::warn!(
                "Playback segment file missing, skip record id={}, path={}",
                record.id,
                record.file_path
            );
        }
    }
    existing
}

pub fn playback_router() -> Router {
    Router::new()
        .route("/list", get(list_playback))
        .route("/device/list", get(list_playback))
        .route("/device/{device_id}/segments", get(list_device_segments))
        .route(
            "/device/{device_id}/segments/delete",
            post(delete_device_segments),
        )
        .route("/device/{device_id}/today", get(list_today_device_segments))
        .route("/playlist/{device_id}", get(playback_playlist))
        .route("/segment-playlist/{id}", get(segment_playlist))
        .route("/segments/delete", post(delete_segments))
        .route("/segment/{id}", get(play_segment))
        .route("/segment/{id}/delete", post(delete_segment))
}

#[derive(Debug, Serialize)]
struct PlaybackSegmentItem {
    id: String,
    start_time: u64,
    duration: f32,
    file_size: usize,
    file_name: String,
    file_path: String,
    video_codec: String,
    video_width: i32,
    video_height: i32,
    video_fps: f32,
    video_bit_rate: i64,
    audio_codec: String,
    audio_sample_rate: i32,
    audio_channels: i32,
    audio_bit_rate: i64,
    create_time: String,
    update_time: String,
}

#[derive(Debug, Serialize)]
struct DevicePlaybackItem {
    device_id: String,
    device_name: String,
    input_type: String,
    segment_count: usize,
}

#[derive(Debug, Deserialize)]
struct PlaybackListQuery {
    page: Option<usize>,
    page_size: Option<usize>,
}

#[derive(Debug, Serialize)]
struct PlaybackListResponse {
    items: Vec<DevicePlaybackItem>,
    page: usize,
    page_size: usize,
    total: usize,
}

#[derive(Debug, Deserialize)]
struct PlaybackSegmentsQuery {
    page: Option<usize>,
    page_size: Option<usize>,
}

async fn list_playback(
    Query(query): Query<PlaybackListQuery>,
) -> ApiJsonResult<PlaybackListResponse> {
    let conn = app_db_conn()?;
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let total = nvr_db::device::count(&conn).await?;
    let devices = nvr_db::device::list_page(page, page_size, &conn).await?;
    let streams = devices
        .iter()
        .map(|device| device.id.clone())
        .collect::<Vec<_>>();
    let segment_counts = nvr_db::record_segment::count_by_streams(&streams, &conn).await?;

    let items = devices
        .into_iter()
        .map(|device| {
            let device_id = device.id.clone();
            DevicePlaybackItem {
                device_id,
                device_name: device.name,
                input_type: device.input_type,
                segment_count: segment_counts.get(&device.id).copied().unwrap_or(0),
            }
        })
        .collect::<Vec<_>>();

    Ok(ok_json(PlaybackListResponse {
        items,
        page,
        page_size,
        total,
    }))
}

#[derive(Debug, Serialize)]
struct PlaybackSegmentsResponse {
    items: Vec<PlaybackSegmentItem>,
    page: usize,
    page_size: usize,
    total: usize,
}

async fn list_device_segments(
    Path(device_id): Path<String>,
    Query(query): Query<PlaybackSegmentsQuery>,
) -> ApiJsonResult<PlaybackSegmentsResponse> {
    let conn = app_db_conn()?;
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(8).clamp(1, 100);
    let total = nvr_db::record_segment::count_by_stream(&device_id, &conn).await?;
    let records = filter_existing_records(
        nvr_db::record_segment::list_by_stream_page(&device_id, page, page_size, &conn).await?,
    )
    .await;
    Ok(ok_json(PlaybackSegmentsResponse {
        items: records
            .into_iter()
            .map(playback_segment_item_from_record)
            .collect(),
        page,
        page_size,
        total,
    }))
}

async fn list_today_device_segments(
    Path(device_id): Path<String>,
) -> ApiJsonResult<Vec<PlaybackSegmentItem>> {
    let conn = app_db_conn()?;
    let now = chrono::Local::now();
    let day_start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid local day start"))?
        .and_local_timezone(chrono::Local)
        .single()
        .ok_or_else(|| anyhow::anyhow!("invalid local timezone conversion"))?
        .timestamp() as u64;
    let day_end = day_start + 24 * 60 * 60;
    let records = filter_existing_records(
        nvr_db::record_segment::list_by_stream_time_range(&device_id, day_start, day_end, &conn)
            .await?,
    )
    .await;
    Ok(ok_json(
        records
            .into_iter()
            .map(playback_segment_item_from_record)
            .collect(),
    ))
}

async fn play_segment(headers: HeaderMap, Path(id): Path<String>) -> ApiResult<Response> {
    let conn = app_db_conn()?;
    let segment = nvr_db::record_segment::get(&id, &conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("record segment not found"))?;
    let content_len = match tokio::fs::metadata(&segment.file_path).await {
        Ok(meta) => meta.len() as usize,
        Err(_) => {
            return Err(
                anyhow::anyhow!("record segment file not found: {}", segment.file_path).into(),
            );
        }
    };
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    let (status, body, content_range, response_len) = if let Some(range_header) = range.as_deref() {
        let (start, end) = match parse_range_header(range_header, content_len) {
            Ok(range) => range,
            Err(()) => {
                let mut response = Response::new(Body::empty());
                *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
                response
                    .headers_mut()
                    .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
                response.headers_mut().insert(
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes */{}", content_len))?,
                );
                return Ok(response);
            }
        };
        // Read only the requested range via seek instead of the whole file:
        // hls.js issues many byte-range requests per segment, and re-reading the
        // entire TS file for each was the main cause of slow playback startup.
        let len = end - start + 1;
        let chunk = read_file_range(&segment.file_path, start as u64, len).await?;
        (
            StatusCode::PARTIAL_CONTENT,
            chunk,
            Some(format!("bytes {}-{}/{}", start, end, content_len)),
            len,
        )
    } else {
        let content = tokio::fs::read(&segment.file_path).await?;
        let len = content.len();
        (StatusCode::OK, content, None, len)
    };

    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("video/mp2t"));
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("inline; filename=\"{}\"", segment.file_name))?,
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&response_len.to_string())?,
    );
    if let Some(content_range) = content_range {
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&content_range)?,
        );
    }
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));

    Ok(response)
}

#[derive(Debug, Serialize)]
struct DeleteSegmentsResult {
    deleted: usize,
}

#[derive(Debug, Deserialize)]
struct DeleteSegmentsRequest {
    ids: Vec<String>,
}

/// Best-effort removal of a segment's file; a missing file is not an error.
async fn remove_segment_file(path: &str) {
    if path.is_empty() {
        return;
    }
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => log::warn!("Failed to delete segment file {}: {:#}", path, err),
    }
}

async fn delete_segment(Path(id): Path<String>) -> ApiJsonResult<DeleteSegmentsResult> {
    let conn = app_db_conn()?;
    let deleted = if let Some(segment) = nvr_db::record_segment::get(&id, &conn).await? {
        remove_segment_file(&segment.file_path).await;
        nvr_db::record_segment::delete(&id, &conn).await?;
        1
    } else {
        0
    };
    Ok(ok_json(DeleteSegmentsResult { deleted }))
}

async fn delete_segments(
    Json(req): Json<DeleteSegmentsRequest>,
) -> ApiJsonResult<DeleteSegmentsResult> {
    let conn = app_db_conn()?;
    let mut deleted = 0;
    for id in req.ids {
        if let Some(segment) = nvr_db::record_segment::get(&id, &conn).await? {
            remove_segment_file(&segment.file_path).await;
            nvr_db::record_segment::delete(&id, &conn).await?;
            deleted += 1;
        }
    }
    Ok(ok_json(DeleteSegmentsResult { deleted }))
}

async fn delete_device_segments(
    Path(device_id): Path<String>,
) -> ApiJsonResult<DeleteSegmentsResult> {
    let conn = app_db_conn()?;
    let records = nvr_db::record_segment::list_by_stream(&device_id, &conn).await?;
    let deleted = records.len();
    for record in &records {
        remove_segment_file(&record.file_path).await;
    }
    nvr_db::record_segment::delete_by_stream(&device_id, &conn).await?;
    Ok(ok_json(DeleteSegmentsResult { deleted }))
}

/// Read `len` bytes starting at `start` from `path` without loading the rest of
/// the file into memory.
async fn read_file_range(path: &str, start: u64, len: usize) -> anyhow::Result<Vec<u8>> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut file = tokio::fs::File::open(path).await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf).await?;
    Ok(buf)
}

fn parse_range_header(range: &str, content_len: usize) -> Result<(usize, usize), ()> {
    if content_len == 0 {
        return Err(());
    }
    let bytes = range.strip_prefix("bytes=").ok_or(())?;
    let (start, end) = bytes.split_once('-').ok_or(())?;

    let (start, end) = match (start.trim(), end.trim()) {
        ("", "") => return Err(()),
        ("", suffix) => {
            let suffix_len = suffix.parse::<usize>().map_err(|_| ())?;
            if suffix_len == 0 {
                return Err(());
            }
            let start = content_len.saturating_sub(suffix_len);
            (start, content_len - 1)
        }
        (start, "") => {
            let start = start.parse::<usize>().map_err(|_| ())?;
            if start >= content_len {
                return Err(());
            }
            (start, content_len - 1)
        }
        (start, end) => {
            let start = start.parse::<usize>().map_err(|_| ())?;
            let end = end.parse::<usize>().map_err(|_| ())?;
            if start > end || start >= content_len {
                return Err(());
            }
            (start, end.min(content_len - 1))
        }
    };

    Ok((start, end))
}

fn detect_ts_packet_size(content: &[u8]) -> Option<usize> {
    [188usize, 192, 204].into_iter().find(|packet_size| {
        if content.len() < packet_size * 3 {
            return false;
        }
        (0..3).all(|index| content[index * packet_size] == 0x47)
    })
}

fn build_even_byterange_segments(
    content_len: usize,
    packet_size: usize,
    total_duration: f32,
) -> Vec<ByteRangeSegment> {
    let segment_count = ((total_duration.max(1.0) as f64) / PLAYBACK_BYTERANGE_SEGMENT_SECONDS)
        .ceil()
        .max(1.0) as usize;
    let aligned_total = content_len - (content_len % packet_size);
    let approx_aligned =
        ((aligned_total / segment_count.max(1)) / packet_size).max(1) * packet_size;

    let mut segments = Vec::new();
    let mut offset = 0usize;
    while offset < aligned_total {
        let remaining = aligned_total - offset;
        let length = if remaining <= approx_aligned {
            remaining
        } else {
            approx_aligned
        };
        segments.push(ByteRangeSegment {
            offset,
            length,
            duration: (total_duration / segment_count.max(1) as f32).max(0.1),
        });
        offset += length;
    }

    if segments.is_empty() {
        segments.push(ByteRangeSegment {
            offset: 0,
            length: content_len,
            duration: total_duration.max(0.1),
        });
    }

    segments
}

async fn segment_playlist(Path(id): Path<String>) -> ApiResult<Response> {
    let conn = app_db_conn()?;
    let segment = nvr_db::record_segment::get(&id, &conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("record segment not found"))?;
    let content_len = match tokio::fs::metadata(&segment.file_path).await {
        Ok(meta) => meta.len() as usize,
        Err(_) => {
            return Err(
                anyhow::anyhow!("record segment file not found: {}", segment.file_path).into(),
            );
        }
    };
    // Detect the TS packet size from a small head read and split by size, instead
    // of reading and PCR-scanning the whole file (which slowed playback opening).
    let head = read_file_range(&segment.file_path, 0, content_len.min(4096)).await?;
    let packet_size = detect_ts_packet_size(&head).unwrap_or(188);
    let sub_segments = build_even_byterange_segments(content_len, packet_size, segment.duration);

    let target_duration = sub_segments
        .iter()
        .map(|item| item.duration.ceil() as i32)
        .max()
        .unwrap_or(1)
        .max(1);
    let mut lines = vec![
        "#EXTM3U".to_string(),
        "#EXT-X-VERSION:4".to_string(),
        format!("#EXT-X-TARGETDURATION:{}", target_duration),
        "#EXT-X-MEDIA-SEQUENCE:0".to_string(),
        "#EXT-X-PLAYLIST-TYPE:VOD".to_string(),
    ];
    for item in sub_segments {
        lines.push(format!("#EXTINF:{:.3},", item.duration));
        lines.push(format!("#EXT-X-BYTERANGE:{}@{}", item.length, item.offset));
        lines.push(format!("/api/playback/segment/{}", segment.id));
    }
    lines.push("#EXT-X-ENDLIST".to_string());
    let body = lines.join("\n");

    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.apple.mpegurl"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn playback_playlist(Path(device_id): Path<String>) -> ApiResult<Response> {
    let conn = app_db_conn()?;
    let device = nvr_db::device::get(&device_id, &conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("device not found"))?;
    let all_records = nvr_db::record_segment::list(&conn).await?;
    let now = chrono::Local::now();
    let day_start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid local day start"))?
        .and_local_timezone(chrono::Local)
        .single()
        .ok_or_else(|| anyhow::anyhow!("invalid local timezone conversion"))?
        .timestamp_millis();
    let day_end = day_start + 24 * 60 * 60 * 1000;

    let mut segments = all_records
        .into_iter()
        .filter(|record| {
            if record.stream != device.id {
                return false;
            }
            let start_ms = (record.start_time as i64) * 1000;
            let end_ms = start_ms + (record.duration * 1000.0) as i64;
            end_ms > day_start && start_ms < day_end
        })
        .collect::<Vec<_>>();
    segments = filter_existing_records(segments).await;
    segments.sort_by_key(|record| record.start_time);

    let target_duration = segments
        .iter()
        .map(|record| record.duration.ceil() as i32)
        .max()
        .unwrap_or(60)
        .max(1);

    let mut lines = vec![
        "#EXTM3U".to_string(),
        "#EXT-X-VERSION:3".to_string(),
        format!("#EXT-X-TARGETDURATION:{}", target_duration),
        "#EXT-X-MEDIA-SEQUENCE:0".to_string(),
        "#EXT-X-PLAYLIST-TYPE:VOD".to_string(),
    ];

    for record in segments {
        lines.push(format!("#EXTINF:{:.3},", record.duration));
        lines.push(format!(
            "#EXT-X-PROGRAM-DATE-TIME:{}",
            chrono::DateTime::from_timestamp(record.start_time as i64, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default()
        ));
        lines.push(format!("/api/playback/segment/{}", record.id));
    }
    lines.push("#EXT-X-ENDLIST".to_string());

    let mut response = Response::new(Body::from(lines.join("\n")));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/vnd.apple.mpegurl"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

fn playback_segment_item_from_record(
    record: nvr_db::record_segment::RecordSegment,
) -> PlaybackSegmentItem {
    PlaybackSegmentItem {
        id: record.id,
        start_time: record.start_time,
        duration: record.duration,
        file_size: record.file_size,
        file_name: record.file_name,
        file_path: record.file_path,
        video_codec: record.video_codec,
        video_width: record.video_width,
        video_height: record.video_height,
        video_fps: record.video_fps,
        video_bit_rate: record.video_bit_rate,
        audio_codec: record.audio_codec,
        audio_sample_rate: record.audio_sample_rate,
        audio_channels: record.audio_channels,
        audio_bit_rate: record.audio_bit_rate,
        create_time: record.create_time.to_rfc3339(),
        update_time: record.update_time.to_rfc3339(),
    }
}
