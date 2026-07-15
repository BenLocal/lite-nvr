use std::time::Duration;

use chrono::{DateTime, Utc};

/// Next epoch-aligned multiple of `period` strictly after `start`.
/// Used for wall-clock-aligned segmentation (e.g. rotate on each minute).
pub fn next_boundary(start: DateTime<Utc>, period: Duration) -> DateTime<Utc> {
    let period_ms = (period.as_millis() as i64).max(1);
    let start_ms = start.timestamp_millis();
    let next_ms = (start_ms.div_euclid(period_ms) + 1) * period_ms;
    DateTime::from_timestamp_millis(next_ms).expect("boundary in range")
}

/// Whether the current packet is a legal point to close a segment.
/// With video present, only a video keyframe may split (a stream-copy that
/// begins mid-GOP is unplayable); audio-only may split on any packet.
pub fn is_split_point(has_video: bool, pkt_is_video: bool, pkt_is_key: bool) -> bool {
    if has_video {
        pkt_is_video && pkt_is_key
    } else {
        true
    }
}

/// Given we are already at a legal split point, decide whether to rotate now.
pub fn should_rotate(
    align_to_wall_clock: bool,
    segment_time: Duration,
    segment_start_wall: DateTime<Utc>,
    now: DateTime<Utc>,
    elapsed_media: Duration,
) -> bool {
    if align_to_wall_clock {
        now >= next_boundary(segment_start_wall, segment_time)
    } else {
        elapsed_media >= segment_time
    }
}

#[cfg(test)]
#[path = "rotation_test.rs"]
mod rotation_test;
