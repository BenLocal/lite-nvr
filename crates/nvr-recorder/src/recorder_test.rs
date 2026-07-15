use super::*;
use crate::config::TrackSelect;

#[test]
fn selects_both_when_present() {
    let s = select_streams(
        [(0, MediaKind::Video), (1, MediaKind::Audio)],
        TrackSelect::Both,
    )
    .unwrap();
    assert_eq!(
        s,
        Selected {
            video: Some(0),
            audio: Some(1)
        }
    );
}

#[test]
fn audio_track_ignores_video_stream() {
    let s = select_streams(
        [(0, MediaKind::Video), (1, MediaKind::Audio)],
        TrackSelect::Audio,
    )
    .unwrap();
    assert_eq!(
        s,
        Selected {
            video: None,
            audio: Some(1)
        }
    );
}

#[test]
fn errors_when_requested_video_absent() {
    assert!(select_streams([(0, MediaKind::Audio)], TrackSelect::Video).is_err());
}

#[test]
fn both_errors_only_when_nothing_present() {
    assert!(select_streams([(0, MediaKind::Other)], TrackSelect::Both).is_err());
    // video-only source with Both is fine (audio None)
    let s = select_streams([(0, MediaKind::Video)], TrackSelect::Both).unwrap();
    assert_eq!(s.video, Some(0));
    assert_eq!(s.audio, None);
}

#[test]
fn backoff_doubles_and_caps() {
    let base = Duration::from_secs(1);
    let max = Duration::from_secs(16);
    assert_eq!(backoff_delay(0, base, max), Duration::from_secs(1));
    assert_eq!(backoff_delay(1, base, max), Duration::from_secs(2));
    assert_eq!(backoff_delay(4, base, max), Duration::from_secs(16));
    assert_eq!(backoff_delay(100, base, max), Duration::from_secs(16));
}
