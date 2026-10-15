use std::time::Duration;

use crate::decoder::{Decoder, DecoderTask};
use crate::frame::RawFrameCmd;
use crate::input::AvInput;
use crate::packet::RawPacketCmd;
use crate::stream::AvStream;
use crate::test_mp4_path;

#[cfg(feature = "rockchip")]
#[test]
fn test_rockchip_hardware_decode() -> anyhow::Result<()> {
    let Some(path) = std::env::var_os("FFMPEG_BUS_RK_TEST_VIDEO") else {
        return Ok(());
    };
    crate::init()?;
    let mut input = AvInput::new(
        path.to_str()
            .ok_or_else(|| anyhow::anyhow!("invalid video path"))?,
        None,
        None,
    )?;
    let stream = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video stream"))?
        .clone();
    let mut decoder = Decoder::new(&stream)?;
    assert!(
        decoder.is_hw && decoder.codec_name.ends_with("_rkmpp"),
        "RKMPP must open on the test board"
    );
    let mut frames = 0;
    while let Some(packet) = input.read_packet() {
        if packet.index() != stream.index() {
            continue;
        }
        decoder.send_packet(packet)?;
        while let Some(frame) = decoder.receive_frame()? {
            if let crate::frame::RawFrame::Video(video) = frame {
                assert_ne!(
                    video.as_video().format(),
                    ffmpeg_next::format::Pixel::DRM_PRIME
                );
                frames += 1;
            }
        }
    }
    decoder.send_eof()?;
    while decoder.receive_frame()?.is_some() {
        frames += 1;
    }
    assert!(frames > 0, "hardware decode must produce frames");
    assert!(
        decoder.is_hw,
        "software fallback does not count as hardware validation"
    );
    Ok(())
}

/// Open test.mp4 and return its video stream plus all of its packets.
fn video_packets() -> Option<(AvInput, AvStream, Vec<RawPacketCmd>)> {
    let path = test_mp4_path();
    if !path.exists() {
        log::warn!("skip: {} not found", path.display());
        return None;
    }
    let mut input = AvInput::new(path.to_str()?, None, None).ok()?;
    let video = input.streams().values().find(|s| s.is_video())?.clone();
    let mut packets = Vec::new();
    while let Some(p) = input.read_packet() {
        if p.index() == video.index() {
            packets.push(RawPacketCmd::Data(p));
        }
    }
    Some((input, video, packets))
}

/// Count data frames until EOF; fails on Lagged (a frame was dropped).
async fn drain_lossless(mut rx: crate::frame::RawFrameReceiver) -> anyhow::Result<usize> {
    let mut n = 0;
    loop {
        match rx.recv().await? {
            RawFrameCmd::Data(_) => {
                n += 1;
                // Slow consumer: the decoder must wait for it, not drop frames.
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            RawFrameCmd::EOF => return Ok(n),
        }
    }
}

/// A lossy subscriber that is never read must not stall the shared decoder,
/// and a slow lossless subscriber must still receive every frame.
#[tokio::test(flavor = "multi_thread")]
async fn test_lossy_subscriber_does_not_block_lossless() -> anyhow::Result<()> {
    let Some((_input, stream, packets)) = video_packets() else {
        return Ok(());
    };
    let total = packets.len();
    let (tx, rx) = tokio::sync::broadcast::channel(4096);
    let task = DecoderTask::new();
    let _idle_lossy = task.subscribe(false);
    let lossless = task.subscribe(true);
    task.start(Decoder::new(&stream)?, rx).await;
    for p in packets {
        assert!(tx.send(p).is_ok());
    }
    assert!(tx.send(RawPacketCmd::EOF).is_ok());

    let got = tokio::time::timeout(Duration::from_secs(20), drain_lossless(lossless)).await??;
    assert_eq!(got, total);
    Ok(())
}

/// When the input channel closes without EOF (input removed), the decoder
/// must wind down and signal EOF instead of hanging forever.
#[tokio::test(flavor = "multi_thread")]
async fn test_decoder_ends_when_input_closes() -> anyhow::Result<()> {
    let Some((_input, stream, _)) = video_packets() else {
        return Ok(());
    };
    let (tx, rx) = tokio::sync::broadcast::channel::<RawPacketCmd>(16);
    let task = DecoderTask::new();
    let mut out = task.subscribe(false);
    task.start(Decoder::new(&stream)?, rx).await;
    drop(tx);

    let msg = tokio::time::timeout(Duration::from_secs(2), out.recv()).await??;
    assert!(matches!(msg, RawFrameCmd::EOF));
    Ok(())
}

/// Dropping the task (bus teardown) stops the decoder even while the input
/// is still open.
#[tokio::test(flavor = "multi_thread")]
async fn test_decoder_stops_on_drop() -> anyhow::Result<()> {
    let Some((_input, stream, _)) = video_packets() else {
        return Ok(());
    };
    let (_tx, rx) = tokio::sync::broadcast::channel::<RawPacketCmd>(16);
    let task = DecoderTask::new();
    let mut out = task.subscribe(false);
    task.start(Decoder::new(&stream)?, rx).await;
    drop(task);

    let msg = tokio::time::timeout(Duration::from_secs(2), out.recv()).await??;
    assert!(matches!(msg, RawFrameCmd::EOF));
    Ok(())
}

/// Feed `packets` one every 20ms (a paced, live-like source), then EOF.
/// Returns the sender task; it finishes when the source is exhausted.
fn spawn_paced_feed(
    packets: Vec<RawPacketCmd>,
) -> (
    tokio::sync::broadcast::Receiver<RawPacketCmd>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, rx) = tokio::sync::broadcast::channel(4096);
    let feed = tokio::spawn(async move {
        for p in packets {
            let _ = tx.send(p);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let _ = tx.send(RawPacketCmd::EOF);
    });
    (rx, feed)
}

async fn first_frame(rx: &mut crate::frame::RawFrameReceiver) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match rx.recv().await {
                Ok(RawFrameCmd::Data(_)) => return Ok(()),
                Ok(RawFrameCmd::EOF) => anyhow::bail!("EOF before any frame"),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }
    })
    .await?
}

/// A bus-managed (auto-stop) decoder stops as soon as its last subscriber
/// leaves, while the source is still producing.
#[tokio::test(flavor = "multi_thread")]
async fn test_auto_stop_decoder_stops_without_subscribers() -> anyhow::Result<()> {
    let Some((_input, stream, packets)) = video_packets() else {
        return Ok(());
    };
    let (rx, feed) = spawn_paced_feed(packets);
    let task = DecoderTask::new_auto_stop();
    let mut sub = task.subscribe(false);
    task.start(Decoder::new(&stream)?, rx).await;
    first_frame(&mut sub).await?;
    drop(sub);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while task.is_running() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !task.is_running(),
        "decoder kept running with no subscribers"
    );
    assert!(!feed.is_finished(), "must stop before the source ends");
    // A late subscriber sees end-of-stream instead of waiting forever.
    assert!(
        task.try_subscribe(false, crate::decoder::FrameDemand::Full)
            .is_none()
    );
    let mut late = task.subscribe(false);
    let r = tokio::time::timeout(Duration::from_secs(1), late.recv()).await?;
    assert!(r.is_err(), "late subscriber must see Closed");
    Ok(())
}

/// Plain decoders (audio mixer, compositor) keep running between subscribers.
#[tokio::test(flavor = "multi_thread")]
async fn test_plain_decoder_survives_subscriber_churn() -> anyhow::Result<()> {
    let Some((_input, stream, packets)) = video_packets() else {
        return Ok(());
    };
    let (rx, _feed) = spawn_paced_feed(packets);
    let task = DecoderTask::new();
    let mut sub = task.subscribe(false);
    task.start(Decoder::new(&stream)?, rx).await;
    first_frame(&mut sub).await?;
    drop(sub);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(task.is_running());
    let mut again = task.subscribe(false);
    first_frame(&mut again).await?;
    Ok(())
}

/// After the decoder has delivered EOF, new subscribers are told the stream
/// is over (Closed) rather than hanging.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscribe_after_eof_is_closed() -> anyhow::Result<()> {
    let Some((_input, stream, _)) = video_packets() else {
        return Ok(());
    };
    let (tx, rx) = tokio::sync::broadcast::channel::<RawPacketCmd>(16);
    let task = DecoderTask::new();
    let mut sub = task.subscribe(false);
    task.start(Decoder::new(&stream)?, rx).await;
    assert!(tx.send(RawPacketCmd::EOF).is_ok());
    let eof = tokio::time::timeout(Duration::from_secs(2), sub.recv()).await??;
    assert!(matches!(eof, RawFrameCmd::EOF));

    let mut late = task.subscribe(false);
    let r = tokio::time::timeout(Duration::from_secs(1), late.recv()).await?;
    assert!(r.is_err(), "late subscriber must see Closed");
    Ok(())
}

/// A hardware decoder that fails at runtime (as QSV does on this kind of
/// machine) is downgraded and then skipped for new decoders.
#[test]
fn test_runtime_failed_hw_decoder_not_reselected() -> anyhow::Result<()> {
    let Some((_input, stream, packets)) = video_packets() else {
        return Ok(());
    };
    let mut decoder = Decoder::new(&stream)?;
    if !decoder.is_hw {
        return Ok(()); // no hardware decoder here: nothing to check
    }
    let name = decoder.codec_name.clone();
    for p in packets {
        if let RawPacketCmd::Data(p) = p {
            decoder.send_packet(p)?;
            while decoder.receive_frame()?.is_some() {}
        }
    }
    if decoder.is_hw {
        return Ok(()); // the hardware decoder works here: no downgrade
    }
    let again = Decoder::new(&stream)?;
    assert_ne!(
        again.codec_name, name,
        "{name} failed at runtime, must be skipped"
    );
    Ok(())
}
#[cfg(feature = "rockchip")]
#[test]
fn test_rockchip_format_negotiation_avoids_hardware_frames() {
    use ffmpeg_next::ffi::AVPixelFormat::*;
    let formats = [AV_PIX_FMT_DRM_PRIME, AV_PIX_FMT_NV12, AV_PIX_FMT_NONE];
    // SAFETY: the test provides a valid NONE-terminated array; context is unused.
    let selected =
        unsafe { super::rockchip_software_format(std::ptr::null_mut(), formats.as_ptr()) };
    assert_eq!(selected, AV_PIX_FMT_NV12);
    let formats = [AV_PIX_FMT_DRM_PRIME, AV_PIX_FMT_NONE];
    // SAFETY: as above, the format array is valid for the entire callback.
    let selected =
        unsafe { super::rockchip_software_format(std::ptr::null_mut(), formats.as_ptr()) };
    assert_eq!(selected, AV_PIX_FMT_NONE);
}

#[test]
fn sdp_parameter_sets_are_prepended_without_losing_packet_properties() -> anyhow::Result<()> {
    use crate::packet::RawPacket;
    let original = [0, 0, 0, 1, 0x65, 1, 2];
    let mut packet = ffmpeg_next::Packet::copy(&original);
    packet.set_pts(Some(90));
    packet.set_dts(Some(80));
    packet.set_duration(30);
    packet.set_stream(2);
    packet.set_flags(ffmpeg_next::packet::Flags::KEY);
    let mut packet = RawPacket::from((packet, ffmpeg_next::Rational(1, 90000)));
    let shared = packet.clone();
    let sets = [0, 0, 0, 1, 0x67, 3];
    super::prepend_parameter_sets(&mut packet, &sets)?;
    assert_eq!(
        packet.packet().data().unwrap(),
        [sets.as_slice(), original.as_slice()].concat()
    );
    assert_eq!(shared.packet().data().unwrap(), original);
    assert_eq!(packet.pts(), Some(90));
    assert_eq!(packet.dts(), Some(80));
    assert_eq!(packet.packet().duration(), 30);
    assert_eq!(packet.index(), 2);
    assert!(packet.is_key());
    Ok(())
}

#[test]
fn test_demand_gate_follows_subscriber_demand() {
    let mut gate = crate::decoder::DemandGate::default();
    // Full from the start: every packet passes, even before a keyframe.
    assert!(gate.admit(false, true));
    // Image-only: keyframes alone reach the decoder.
    assert!(gate.admit(true, false));
    assert!(!gate.admit(false, false));
    // A Full subscriber arrives mid-GOP: wait for the next keyframe.
    assert!(!gate.admit(false, true), "upgrade must start on a keyframe");
    assert!(gate.admit(true, true));
    assert!(gate.admit(false, true), "full decode passes every packet");
    // Last Full subscriber leaves: back to keyframes at once.
    assert!(!gate.admit(false, false));
    assert!(gate.admit(true, false));
}

#[test]
fn test_intra_only_codecs_skip_the_keyframe_gate() {
    use ffmpeg_next::codec::Id;
    assert!(crate::decoder::intra_only(Id::MJPEG));
    assert!(crate::decoder::intra_only(Id::WRAPPED_AVFRAME));
    assert!(!crate::decoder::intra_only(Id::H264));
    assert!(!crate::decoder::intra_only(Id::HEVC));
}

/// Decoding keyframes alone, a keyframe's picture must come out without
/// waiting for the next packet: decoders with reorder delay (B-frames) or
/// frame threading otherwise hold it back for a whole GOP.
#[tokio::test(flavor = "multi_thread")]
async fn test_image_keyframe_is_output_without_next_packet() -> anyhow::Result<()> {
    let Some((_input, stream, packets)) = video_packets() else {
        return Ok(());
    };
    let key = packets
        .into_iter()
        .find(|p| matches!(p, RawPacketCmd::Data(p) if p.is_key()))
        .ok_or_else(|| anyhow::anyhow!("no keyframe"))?;
    let (tx, rx) = tokio::sync::broadcast::channel(16);
    let task = DecoderTask::new_auto_stop();
    let mut image = task
        .try_subscribe(false, crate::decoder::FrameDemand::Image)
        .ok_or_else(|| anyhow::anyhow!("decoder closed"))?;
    task.start(Decoder::new(&stream)?, rx).await;
    // Only the keyframe; the source stays open (no EOF to flush the decoder).
    tx.send(key)
        .map_err(|_| anyhow::anyhow!("decoder relay gone"))?;
    let frame = tokio::time::timeout(Duration::from_secs(2), image.recv()).await;
    assert!(
        matches!(frame, Ok(Ok(RawFrameCmd::Data(_)))),
        "keyframe picture held back by the decoder"
    );
    drop(tx);
    Ok(())
}
