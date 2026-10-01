use ffmpeg_next::Rational;

use super::frame_rate;

#[test]
fn test_frame_rate_prefers_average() {
    assert_eq!(
        frame_rate(Rational(25, 1), Rational(50, 1)),
        Rational(25, 1)
    );
}

/// Raw video and many RTSP cameras leave avg_frame_rate at 0/0.
#[test]
fn test_frame_rate_falls_back_to_real_rate() {
    assert_eq!(frame_rate(Rational(0, 0), Rational(10, 1)), Rational(10, 1));
    assert_eq!(frame_rate(Rational(0, 1), Rational(30, 1)), Rational(30, 1));
}
