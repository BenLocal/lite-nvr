use std::path::Path;
use std::time::Duration;

use crate::decoder::{Decoder, DecoderTask};
use crate::encoder::{Encoder, EncoderTask, Settings};
use crate::input::AvInput;
use crate::packet::RawPacketCmd;

/// A bus-managed (auto-stop) encoder stops as soon as its last output
/// subscriber leaves, while frames are still arriving.
#[tokio::test(flavor = "multi_thread")]
async fn test_auto_stop_encoder_stops_without_subscribers() -> anyhow::Result<()> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test.mp4");
    if !path.exists() {
        return Ok(());
    }
    let mut input = AvInput::new(path.to_str().unwrap_or_default(), None, None)?;
    let stream = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video stream"))?
        .clone();
    let mut packets = Vec::new();
    while let Some(p) = input.read_packet() {
        if p.index() == stream.index() {
            packets.push(RawPacketCmd::Data(p));
        }
    }

    // Paced source: one packet every 20ms (~1s for the 50 frames).
    let (tx, rx) = tokio::sync::broadcast::channel(4096);
    let feed = tokio::spawn(async move {
        for p in packets {
            let _ = tx.send(p);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let _ = tx.send(RawPacketCmd::EOF);
    });
    let decoder = DecoderTask::new();
    let frames = decoder.subscribe(true);
    decoder.start(Decoder::new(&stream)?, rx).await;

    let settings = Settings {
        width: 160,
        height: 120,
        ..Settings::default()
    };
    let task = EncoderTask::new_auto_stop();
    let mut out = task.subscribe();
    task.start(Encoder::new(&stream, settings, None)?, frames, false)
        .await;

    let first = tokio::time::timeout(Duration::from_secs(5), out.recv()).await??;
    assert!(matches!(first, RawPacketCmd::Data(_)));
    drop(out);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while task.is_running() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !task.is_running(),
        "encoder kept running with no subscribers"
    );
    assert!(!feed.is_finished(), "must stop before the source ends");
    assert!(task.try_subscribe().is_none());
    Ok(())
}

/// A hardware encoder that fails on real frames is replaced by a software one
/// and the failing frame is replayed: every frame still comes out encoded.
#[test]
fn test_hardware_encoder_runtime_failure_falls_back_to_software() -> anyhow::Result<()> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test.mp4");
    if !path.exists() {
        return Ok(());
    }
    let mut input = AvInput::new(path.to_str().unwrap_or_default(), None, None)?;
    let stream = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video stream"))?
        .clone();
    let settings = Settings {
        width: 160,
        height: 120,
        ..Settings::default()
    };
    let mut encoder = Encoder::new(&stream, settings, None)?;
    // Pretend the selected codec is hardware that breaks on the first frame.
    encoder.is_hw = true;
    encoder.inject_hw_failure = true;

    let mut decoder = Decoder::new(&stream)?;
    let (mut frames, mut packets) = (0, 0);
    let drain = |encoder: &mut Encoder| -> anyhow::Result<usize> {
        let mut n = 0;
        while encoder.encoder_receive_packet()?.is_some() {
            n += 1;
        }
        Ok(n)
    };
    while let Some(packet) = input.read_packet() {
        if packet.index() != stream.index() {
            continue;
        }
        decoder.send_packet(packet)?;
        while let Some(frame) = decoder.receive_frame()? {
            encoder.send_frame(frame)?;
            frames += 1;
            packets += drain(&mut encoder)?;
        }
    }
    decoder.send_eof()?;
    while let Some(frame) = decoder.receive_frame()? {
        encoder.send_frame(frame)?;
        frames += 1;
        packets += drain(&mut encoder)?;
    }
    encoder.send_eof()?;
    packets += drain(&mut encoder)?;

    assert!(!encoder.is_hw, "encoder must have been downgraded");
    assert_eq!(frames, 50);
    assert_eq!(packets, frames, "the failed frame is replayed, none lost");
    Ok(())
}

fn audio_frame(pts: i64, samples: usize) -> ffmpeg_next::frame::Audio {
    let fmt = ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Planar);
    let mut frame =
        ffmpeg_next::frame::Audio::new(fmt, samples, ffmpeg_next::ChannelLayout::STEREO);
    frame.set_rate(48_000);
    frame.set_pts(Some(pts));
    frame
}

fn resampler() -> anyhow::Result<super::AudioResampler> {
    let fmt = ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Planar);
    super::AudioResampler::new(
        &audio_frame(0, 1024),
        48_000,
        fmt,
        ffmpeg_next::ChannelLayout::STEREO,
        1024,
    )
}

/// Lost input (a PTS jump) is bridged with silence: the frames after the gap
/// keep their source timing instead of being pulled earlier.
#[test]
fn test_audio_gap_is_bridged_with_silence() -> anyhow::Result<()> {
    let mut r = resampler()?;
    r.push(&audio_frame(0, 1024))?;
    r.push(&audio_frame(1024, 1024))?;
    // 8 frames (~170ms) lost, then the stream resumes.
    r.push(&audio_frame(1024 * 10, 1024))?;
    let mut out = r.drain()?;
    out.extend(r.flush()?);
    let last = out.last().ok_or_else(|| anyhow::anyhow!("no output"))?;
    let end = last.pts().unwrap_or(0) + last.samples() as i64;
    assert_eq!(end, 1024 * 11, "timeline must include the 8 lost frames");
    Ok(())
}

/// Small jitter is not a gap; a huge jump moves the timeline without
/// inserting seconds of silence.
#[test]
fn test_audio_jitter_ignored_and_big_jump_moves_timeline() -> anyhow::Result<()> {
    let mut r = resampler()?;
    r.push(&audio_frame(0, 1024))?;
    r.push(&audio_frame(1024 + 100, 1024))?; // ~2ms jitter
    r.push(&audio_frame(48_000 * 60, 1024))?; // 60s jump
    let mut out = r.drain()?;
    out.extend(r.flush()?);
    let total: usize = out.iter().map(|f| f.samples()).sum();
    assert!(total < 48_000, "no long silence inserted: {total} samples");
    let last = out.last().ok_or_else(|| anyhow::anyhow!("no output"))?;
    assert!(
        last.pts().unwrap_or(0) >= 48_000 * 60 - 2048,
        "timeline moved forward"
    );
    Ok(())
}

#[test]
fn test_gop_frames_follows_frame_rate() {
    use ffmpeg_next::Rational;

    use super::{AUTO_GOP_SECS, gop_frames};
    assert_eq!(gop_frames(0, Rational(25, 1)), 25 * AUTO_GOP_SECS);
    assert_eq!(gop_frames(0, Rational(60, 1)), 60 * AUTO_GOP_SECS);
    assert_eq!(gop_frames(0, Rational(30000, 1001)), 60, "29.97fps rounds");
    assert_eq!(gop_frames(0, Rational(0, 0)), 50, "unknown rate falls back");
    assert_eq!(
        gop_frames(12, Rational(25, 1)),
        12,
        "explicit interval wins"
    );
}

#[test]
fn test_pick_sample_rate() {
    use super::pick_sample_rate;
    let opus = [48_000, 24_000, 16_000, 12_000, 8_000];
    assert_eq!(pick_sample_rate(44_100, &opus), 48_000);
    assert_eq!(pick_sample_rate(22_050, &opus), 24_000);
    assert_eq!(pick_sample_rate(16_000, &opus), 16_000, "supported: kept");
    assert_eq!(pick_sample_rate(44_100, &[]), 44_100, "no constraint");
    assert_eq!(
        pick_sample_rate(20_000, &[16_000, 24_000]),
        24_000,
        "tie → higher"
    );
}
