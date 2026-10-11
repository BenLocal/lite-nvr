use super::*;

#[test]
fn settings_map_to_distinct_video_and_audio_outputs() {
    let cfg = Settings {
        video_codec: Some(VideoCodec::H265),
        width: Some(640),
        height: Some(360),
        fps: Some(15),
        max_bitrate: Some(500_000),
        audio_codec: Some(AudioCodec::Opus),
        sample_rate: Some(24000),
    };
    let mut outputs = vec![OutputConfig::new(
        media_pipe_core::OutputDest::Network {
            url: "rtsp://localhost/test".into(),
            format: "rtsp".into(),
        },
        None,
    )];
    outputs.push(outputs[0].clone());
    apply(&mut outputs, Some(&cfg));
    let video = outputs[0].encode.as_ref().unwrap();
    assert_eq!(video.codec, "hevc");
    assert_eq!(video.fps, Some(15));
    assert_eq!(video.max_bitrate, Some(500_000));
    assert!(video.downscale_only);
    let audio = outputs[1].encode.as_ref().unwrap();
    assert_eq!(audio.codec, "libopus");
    assert_eq!(audio.sample_rate, Some(24000));
}
