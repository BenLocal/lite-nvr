use super::*;
use crate::encoder::{AudioSettings, Encoder};
use crate::packet::RawPacketCmd;
use ffmpeg_next::{Rational, codec::Parameters};

fn audio_stream() -> AvStream {
    crate::init().unwrap();
    let template = AvStream::new(0, Parameters::new(), Rational(1, 48_000), Rational(0, 1));
    Encoder::new_audio(
        &template,
        AudioSettings {
            sample_rate: Some(48_000),
            channels: Some(2),
            bitrate: Some(128_000),
            ..Default::default()
        },
        None,
    )
    .unwrap()
    .output_stream(0)
}

fn packet(pts: i64) -> RawPacket {
    let mut p = ffmpeg_next::Packet::copy(&[1, 2, 3]);
    p.set_pts(Some(pts));
    p.set_dts(Some(pts));
    p.set_duration(1024);
    (p, Rational(1, 48_000)).into()
}

#[tokio::test]
async fn shared_audio_subscribers_receive_the_same_encoded_payload() {
    let (tx, _) = tokio::sync::broadcast::channel(8);
    let audio = SharedEncodedAudio::new(audio_stream(), tx.clone()).unwrap();
    let mut a = audio.subscribe();
    let mut b = audio.subscribe();
    assert!(tx.send(RawPacketCmd::Data(packet(48_000))).is_ok());
    let (RawPacketCmd::Data(a), RawPacketCmd::Data(b)) =
        (a.recv().await.unwrap(), b.recv().await.unwrap())
    else {
        panic!("expected encoded packets");
    };
    assert_eq!(
        a.packet().data().unwrap().as_ptr(),
        b.packet().data().unwrap().as_ptr()
    );
    assert_eq!(audio.stream().sample_rate(), 48_000);
    assert_eq!(audio.stream().channels(), 2);
}

#[test]
fn consumer_timestamps_are_independent_and_do_not_modify_shared_packets() {
    let original = packet(480_000);
    let mut a = AudioTimeline::new(Rational(1, 1000), 2000);
    let mut b = AudioTimeline::new(Rational(1, 90_000), 900_000);
    let mapped_a = a.map(original.clone(), 2).unwrap();
    let mapped_b = b.map(original.clone(), 3).unwrap();
    assert_eq!(mapped_a.pts(), Some(2000));
    assert_eq!(mapped_b.pts(), Some(900_000));
    assert_eq!(mapped_a.index(), 2);
    assert_eq!(mapped_b.index(), 3);
    assert_eq!(original.pts(), Some(480_000));
    assert_eq!(
        mapped_a.packet().data().unwrap().as_ptr(),
        original.packet().data().unwrap().as_ptr()
    );
    let next = a.map(packet(528_000), 2).unwrap();
    assert_eq!(next.pts(), Some(3000));
}

#[test]
fn timeline_preserves_gaps_instead_of_compressing_dropped_audio() {
    let mut timeline = AudioTimeline::new(Rational(1, 48_000), 96_000);
    assert_eq!(timeline.map(packet(-1024), 1).unwrap().pts(), Some(96_000));
    assert_eq!(
        timeline.map(packet(48_000), 1).unwrap().pts(),
        Some(145_024)
    );
}
