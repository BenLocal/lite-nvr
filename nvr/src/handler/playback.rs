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

fn detect_ts_packet_size(content: &[u8]) -> Option<usize> {
    [188usize, 192, 204].into_iter().find(|packet_size| {
        if content.len() < packet_size * 3 {
            return false;
        }
        (0..3).all(|index| content[index * packet_size] == 0x47)
    })
}

fn parse_pcr_seconds(packet: &[u8]) -> Option<f64> {
    if packet.len() < 12 || packet.first().copied()? != 0x47 {
        return None;
    }
    let adaptation_control = (packet[3] >> 4) & 0x03;
    if adaptation_control != 0b10 && adaptation_control != 0b11 {
        return None;
    }
    let adaptation_len = packet[4] as usize;
    if adaptation_len < 7 || 5 + adaptation_len > packet.len() {
        return None;
    }
    if packet[5] & 0x10 == 0 {
        return None;
    }

    let pcr_base = ((packet[6] as u64) << 25)
        | ((packet[7] as u64) << 17)
        | ((packet[8] as u64) << 9)
        | ((packet[9] as u64) << 1)
        | ((packet[10] as u64) >> 7);
    let pcr_ext = (((packet[10] & 0x01) as u64) << 8) | packet[11] as u64;
    Some(((pcr_base * 300) + pcr_ext) as f64 / 27_000_000.0)
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
    let approx_aligned = ((aligned_total / segment_count.max(1)) / packet_size).max(1) * packet_size;

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

fn build_ts_byterange_segments(content: &[u8], total_duration: f32) -> Vec<ByteRangeSegment> {
    let Some(packet_size) = detect_ts_packet_size(content) else {
        return vec![ByteRangeSegment {
            offset: 0,
            length: content.len(),
            duration: total_duration.max(0.1),
        }];
    };

    let aligned_len = content.len() - (content.len() % packet_size);
    let mut pcr_points = Vec::<(usize, f64)>::new();
    let mut offset = 0usize;
    while offset + packet_size <= aligned_len {
        if let Some(seconds) = parse_pcr_seconds(&content[offset..offset + packet_size]) {
            pcr_points.push((offset, seconds));
        }
        offset += packet_size;
    }

    if pcr_points.len() < 2 {
        return build_even_byterange_segments(content.len(), packet_size, total_duration);
    }

    let first_pcr = pcr_points[0].1;
    let last_pcr = pcr_points[pcr_points.len() - 1].1;
    let usable_duration = if last_pcr > first_pcr {
        (last_pcr - first_pcr) as f32
    } else {
        total_duration
    }
    .max(0.1);

    let mut boundaries = vec![0usize];
    let mut next_target = PLAYBACK_BYTERANGE_SEGMENT_SECONDS;
    for (packet_offset, seconds) in pcr_points.iter().copied() {
        let relative = seconds - first_pcr;
        if relative + 0.001 >= next_target {
            if packet_offset > *boundaries.last().unwrap_or(&0) {
                boundaries.push(packet_offset);
            }
            next_target += PLAYBACK_BYTERANGE_SEGMENT_SECONDS;
        }
    }
    if *boundaries.last().unwrap_or(&0) != aligned_len {
        boundaries.push(aligned_len);
    }

    let mut segments = Vec::new();
    for window in boundaries.windows(2) {
        let start = window[0];
        let end = window[1];
        if end <= start {
            continue;
        }
        let duration = if usable_duration > 0.0 {
            (((end - start) as f64 / aligned_len.max(1) as f64) * usable_duration as f64) as f32
        } else {
            PLAYBACK_BYTERANGE_SEGMENT_SECONDS as f32
        };
        segments.push(ByteRangeSegment {
            offset: start,
            length: end - start,
            duration: duration.max(0.1),
        });
    }

    if segments.is_empty() {
        return build_even_byterange_segments(content.len(), packet_size, total_duration);
    }

    segments
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

    let content = tokio::fs::read(&segment.file_path).await?;
    let sub_segments = build_ts_byterange_segments(&content, segment.duration);

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
