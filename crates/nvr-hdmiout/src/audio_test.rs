use super::*;

#[test]
fn rejects_invalid_audio_configuration_before_opening_device() {
    for config in [
        AudioConfig {
            device: "".into(),
            ..Default::default()
        },
        AudioConfig {
            device: "bad\0device".into(),
            ..Default::default()
        },
        AudioConfig {
            sample_rate: 0,
            ..Default::default()
        },
        AudioConfig {
            channels: 0,
            ..Default::default()
        },
        AudioConfig {
            channels: 3,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            AudioOutput::open(&config),
            Err(Error::InvalidAudio)
        ));
    }
}

#[cfg(target_os = "linux")]
#[test]
fn pcm_null_sink_validates_frames_and_closes_immediately() {
    let mut output = AudioOutput::open(&AudioConfig {
        device: "null".into(),
        ..Default::default()
    })
    .unwrap();
    assert!(matches!(output.write(&[1]), Err(Error::InvalidAudio)));
    output.write(&[0; 960]).unwrap();
    output.close().unwrap();
    output.close().unwrap();
    assert!(matches!(output.write(&[0; 960]), Err(Error::Closed)));
}
