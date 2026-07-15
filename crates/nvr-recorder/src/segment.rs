//! One output file over an `AvOutput`, plus the pure timestamp math used to
//! reset each segment to a ~0 origin (emulating ffmpeg `-reset_timestamps 1`).

/// Rescale a timestamp in `tb` (num/den seconds per tick) to microseconds.
pub(crate) fn tb_to_us(ts: i64, tb_num: i32, tb_den: i32) -> i64 {
    if tb_den == 0 {
        return 0;
    }
    (ts as i128 * tb_num as i128 * 1_000_000 / tb_den as i128) as i64
}

/// Rescale a microsecond value into ticks of `tb` (num/den seconds per tick).
pub(crate) fn us_to_tb(us: i64, tb_num: i32, tb_den: i32) -> i64 {
    if tb_num == 0 {
        return 0;
    }
    (us as i128 * tb_den as i128 / (tb_num as i128 * 1_000_000)) as i64
}

/// Segment duration in seconds from first/last primary-stream PTS (microseconds).
pub(crate) fn duration_seconds(first_us: i64, last_us: i64) -> f64 {
    (last_us - first_us).max(0) as f64 / 1_000_000.0
}

#[cfg(test)]
#[path = "segment_test.rs"]
mod segment_test;
