use super::*;

fn t(ms: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(ms).unwrap()
}

#[test]
fn boundary_rounds_up_to_next_multiple() {
    // period 60s, start at 00:00:12.000 -> next boundary 00:01:00.000
    assert_eq!(next_boundary(t(12_000), Duration::from_secs(60)), t(60_000));
}

#[test]
fn boundary_is_strictly_after_when_on_multiple() {
    assert_eq!(
        next_boundary(t(60_000), Duration::from_secs(60)),
        t(120_000)
    );
}

#[test]
fn split_point_with_video_requires_video_keyframe() {
    assert!(is_split_point(true, true, true));
    assert!(!is_split_point(true, true, false));
    assert!(!is_split_point(true, false, true));
}

#[test]
fn split_point_audio_only_is_anywhere() {
    assert!(is_split_point(false, false, false));
}

#[test]
fn rotate_on_media_duration() {
    let time = Duration::from_secs(10);
    assert!(!should_rotate(
        false,
        time,
        t(0),
        t(0),
        Duration::from_secs(9)
    ));
    assert!(should_rotate(
        false,
        time,
        t(0),
        t(0),
        Duration::from_secs(10)
    ));
}

#[test]
fn rotate_on_wall_clock_boundary() {
    let time = Duration::from_secs(60);
    let start = t(12_000); // boundary at 60_000
    assert!(!should_rotate(
        true,
        time,
        start,
        t(59_000),
        Duration::from_secs(47)
    ));
    assert!(should_rotate(
        true,
        time,
        start,
        t(60_000),
        Duration::from_secs(48)
    ));
}
