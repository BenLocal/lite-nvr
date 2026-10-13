use super::*;

#[test]
fn detaching_a_packet_header_preserves_shared_payload_ownership() -> anyhow::Result<()> {
    let mut original = ffmpeg_next::Packet::copy(&[1, 2, 3]);
    original.set_pts(Some(7));
    let original = RawPacket::from((original, Rational(1, 48000)));
    let mut output = original.clone();
    output.get_mut()?.set_pts(Some(9));
    assert_eq!(original.pts(), Some(7));
    assert_eq!(output.pts(), Some(9));
    assert_eq!(
        original.packet().data().unwrap().as_ptr(),
        output.packet().data().unwrap().as_ptr()
    );
    drop(original);
    assert_eq!(output.packet().data(), Some([1, 2, 3].as_slice()));
    Ok(())
}

#[test]
fn wrapped_video_frame_survives_header_detach_and_original_drop() -> anyhow::Result<()> {
    crate::init()?;
    let mut input =
        crate::input::AvInput::open("testsrc2=size=64x48:rate=1", Some("lavfi"), None, None)?;
    let stream = input
        .streams()
        .values()
        .find(|stream| stream.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video stream"))?
        .clone();
    let original = input
        .read_packet()
        .ok_or_else(|| anyhow::anyhow!("no wrapped frame"))?;
    let mut output = original.clone();
    output.get_mut()?.set_stream(stream.index());
    drop(original);
    drop(input);
    let mut decoder = ffmpeg_next::codec::Context::from_parameters(stream.parameters().clone())?
        .decoder()
        .video()?;
    decoder.send_packet(output.packet())?;
    let mut frame = ffmpeg_next::frame::Video::empty();
    decoder.receive_frame(&mut frame)?;
    assert_eq!((frame.width(), frame.height()), (64, 48));
    assert!(!frame.data(0).is_empty());
    Ok(())
}
