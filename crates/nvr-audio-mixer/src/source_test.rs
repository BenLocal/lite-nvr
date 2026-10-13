use super::*;
use ffmpeg_bus::frame::RawFrame;

fn frame(value: i16) -> RawFrameCmd {
    let mut audio = ffmpeg_next::frame::Audio::new(
        ffmpeg_next::format::Sample::I16(ffmpeg_next::format::sample::Type::Packed),
        1024,
        ffmpeg_next::ChannelLayout::STEREO,
    );
    audio.set_rate(48_000);
    audio.set_pts(Some(0));
    for sample in audio.plane_mut::<(i16, i16)>(0) {
        *sample = (value, value);
    }
    RawFrameCmd::Data(RawFrame::Audio(audio.into()))
}

#[tokio::test]
async fn source_reconnect_keeps_the_program_receiver_alive() {
    ffmpeg_bus::init().unwrap();
    let source = AudioSource::shared("camera");
    let mut output = source.subscribe();
    let (first_tx, first) = tokio::sync::broadcast::channel(4);
    source.attach(first).unwrap();
    assert!(first_tx.send(frame(100)).is_ok());
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(2), output.recv())
            .await
            .unwrap()
            .unwrap(),
        RawFrameCmd::Data(_)
    ));
    assert!(first_tx.send(RawFrameCmd::EOF).is_ok());
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    assert!(!source.ready());
    let (second_tx, second) = tokio::sync::broadcast::channel(4);
    source.attach(second).unwrap();
    assert!(second_tx.send(frame(200)).is_ok());
    let RawFrameCmd::Data(RawFrame::Audio(audio)) =
        tokio::time::timeout(std::time::Duration::from_secs(2), output.recv())
            .await
            .unwrap()
            .unwrap()
    else {
        panic!("reconnected source did not deliver PCM")
    };
    assert_eq!(audio.as_audio().plane::<(i16, i16)>(0)[0], (200, 200));
    source.shutdown().await;
}
