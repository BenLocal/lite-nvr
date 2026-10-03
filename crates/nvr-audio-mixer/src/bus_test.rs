use std::time::Duration;

use ffmpeg_bus::frame::RawFrameCmd;
use ffmpeg_bus::input::AvInput;
use ffmpeg_next::Rational;

use super::*;

/// Frames produced by the mixer use output samples as PTS regardless of the
/// source template's time base. No source should cause extra silence packets.
#[tokio::test(flavor = "multi_thread")]
async fn test_mixed_audio_timing_is_independent_of_template() -> anyhow::Result<()> {
    ffmpeg_bus::init()?;
    for (rate, time_base) in [
        (8_000, Rational(1, 8_000)),
        (44_100, Rational(1, 44_100)),
        (48_000, Rational(1, 90_000)),
    ] {
        let input = AvInput::new(&format!("sine=sample_rate={rate}"), Some("lavfi"), None)?;
        let source = input.streams().values().find(|s| s.is_audio()).unwrap();
        let template = AvStream::new(
            source.index(),
            source.parameters().clone(),
            time_base,
            source.rate(),
        );
        let mut encoder = MixBus::new_encoder(&template)?;
        let mixer = DynamicMixerTask::new(SAMPLE_RATE);
        let mut mixed = mixer.subscribe();
        mixer.start();
        let mut pts = Vec::new();
        for i in 0..3 {
            let RawFrameCmd::Data(frame) =
                tokio::time::timeout(Duration::from_secs(5), mixed.recv()).await??
            else {
                panic!("mixer ended before the third frame");
            };
            if let ffmpeg_bus::frame::RawFrame::Audio(audio) = &frame {
                assert_eq!(audio.pts(), Some(i * 1024));
                assert_eq!(audio.as_audio().rate(), SAMPLE_RATE);
            } else {
                panic!("mixer emitted a video frame");
            }
            encoder.send_frame(frame)?;
            while let Some(packet) = encoder.encoder_receive_packet()? {
                pts.push(packet.pts().unwrap());
            }
        }
        mixer.cancel();
        encoder.send_eof()?;
        while let Some(packet) = encoder.encoder_receive_packet()? {
            pts.push(packet.pts().unwrap());
        }
        assert_eq!(
            pts,
            [-1024, 0, 1024, 2048],
            "only three audio frames and one priming packet for template {rate}Hz/{time_base:?}"
        );
        assert_eq!(encoder.output_stream(0).time_base(), Rational(1, 48_000));
    }
    Ok(())
}
