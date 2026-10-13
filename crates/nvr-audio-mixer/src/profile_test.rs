use super::*;

#[test]
fn profile_rejects_silent_sample_rate_fallback_and_stereo_downmix() {
    assert!(AudioProfile::default().validate().is_ok());
    assert!(
        AudioProfile {
            codec: AudioCodec::Opus,
            sample_rate: 44100,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        AudioProfile {
            channels: 1,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        AudioProfile {
            bitrate: 0,
            ..Default::default()
        }
        .validate()
        .is_err()
    );
}
