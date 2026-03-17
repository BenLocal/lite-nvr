use axum::{
    Router,
    body::Body,
    extract::{Path, Query},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
    routing::get,
};
use serde::{Deserialize, Serialize};

use crate::{
    db::app_db_conn,
    handler::{ApiJsonResult, ApiResult, ok_json},
};

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
        .route("/device/{device_id}/today", get(list_today_device_segments))
        .route("/playlist/{device_id}", get(playback_playlist))
        .route("/segment-playlist/{id}", get(segment_playlist))
        .route("/segment/{id}", get(play_segment))
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
    if !segment_file_exists(&segment.file_path).await {
        return Err(anyhow::anyhow!(
            "record segment file not found: {}",
            segment.file_path
        )
        .into());
    }
    let content = tokio::fs::read(&segment.file_path).await?;
    let content_len = content.len();
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
        (
            StatusCode::PARTIAL_CONTENT,
            content[start..=end].to_vec(),
            Some(format!("bytes {}-{}/{}", start, end, content_len)),
            end - start + 1,
        )
    } else {
        (StatusCode::OK, content, None, content_len)
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

fn parse_range_header(range: &str, content_len: usize) -> Result<(usize, usize), ()> {
    if content_len == 0 {
        return Err(());
    }
    let bytes = range
        .strip_prefix("bytes=")
        .ok_or(())?;
    let (start, end) = bytes
        .split_once('-')
        .ok_or(())?;

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

async fn segment_playlist(Path(id): Path<String>) -> ApiResult<Response> {
    let conn = app_db_conn()?;
    let segment = nvr_db::record_segment::get(&id, &conn)
        .await?
        .ok_or_else(|| anyhow::anyhow!("record segment not found"))?;
    if !segment_file_exists(&segment.file_path).await {
        return Err(anyhow::anyhow!(
            "record segment file not found: {}",
            segment.file_path
        )
        .into());
    }

    let body = format!(
        "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{target}\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:{duration:.3},\n/api/playback/segment/{id}\n#EXT-X-ENDLIST\n",
        target = (segment.duration.ceil() as i32).max(1),
        duration = segment.duration,
        id = segment.id,
    );

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
