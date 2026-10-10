use std::time::Duration;

use crate::decoder::{Decoder, DecoderTask};
use crate::encoder::{Encoder, EncoderTask, Settings};
use crate::input::AvInput;
use crate::packet::RawPacketCmd;

#[cfg(feature = "rockchip")]
#[test]
fn test_rockchip_hardware_encode() -> anyhow::Result<()> {
    if std::env::var_os("FFMPEG_BUS_RK_TEST_VIDEO").is_none() {
        return Ok(());
    }
    crate::init()?;
    let input = AvInput::new("testsrc2=size=320x240:rate=25", Some("lavfi"), None)?;
    let stream = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no test video stream"))?;
    let name =
        std::env::var("FFMPEG_BUS_RK_TEST_ENCODER").unwrap_or_else(|_| "h264_rkmpp".to_string());
    for name in [name.as_str()] {
        let mut encoder = Encoder::new(
            stream,
            Settings {
                width: 320,
                height: 240,
                codec: Some(name.to_string()),
                pixel_format: ffmpeg_next::format::Pixel::YUV420P,
                ..Settings::default()
            },
            None,
        )?;
        assert_eq!(
            encoder.codec_name, name,
            "software fallback is not a hardware test"
        );
        let mut packets = 0;
        for index in 0..25 {
            let mut frame =
                ffmpeg_next::frame::Video::new(ffmpeg_next::format::Pixel::YUV420P, 320, 240);
            for plane in 0..3 {
                frame.data_mut(plane).fill(128);
            }
            frame.set_pts(Some(index));
            encoder.send_frame(crate::frame::RawFrame::Video(frame.into()))?;
            while encoder.encoder_receive_packet()?.is_some() {
                packets += 1;
            }
        }
        encoder.send_eof()?;
        while encoder.encoder_receive_packet()?.is_some() {
            packets += 1;
        }
        assert!(
            encoder.is_hw && packets > 0,
            "RKMPP must produce encoded packets"
        );
    }
    Ok(())
}

/// A bus-managed (auto-stop) encoder stops as soon as its last output
/// subscriber leaves, while frames are still arriving.
#[tokio::test(flavor = "multi_thread")]
async fn test_auto_stop_encoder_stops_without_subscribers() -> anyhow::Result<()> {
    let path = crate::test_mp4_path();
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
    let path = crate::test_mp4_path();
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

/// Fill an audio frame with silence. `Audio::new` leaves samples
/// uninitialized (garbage floats make AAC fail with EINVAL or crawl), and
/// ffmpeg-next's `data_mut(i)` is empty for planar planes i > 0 (FFmpeg only
/// sets `linesize[0]` for audio), so zeroing via `data_mut` misses channels.
fn fill_silence(frame: &mut ffmpeg_next::frame::Audio) {
    let fmt: ffmpeg_next::ffi::AVSampleFormat = frame.format().into();
    let (samples, channels) = (frame.samples() as i32, i32::from(frame.channels()));
    // SAFETY: the frame was allocated by `Audio::new` for exactly this
    // format, sample count and channel count; extended_data has one plane per
    // channel (planar) or one interleaved plane.
    unsafe {
        ffmpeg_next::ffi::av_samples_set_silence(
            (*frame.as_mut_ptr()).extended_data,
            0,
            samples,
            channels,
            fmt,
        );
    }
}

fn audio_frame(pts: i64, samples: usize) -> ffmpeg_next::frame::Audio {
    let fmt = ffmpeg_next::format::Sample::F32(ffmpeg_next::format::sample::Type::Planar);
    let mut frame =
        ffmpeg_next::frame::Audio::new(fmt, samples, ffmpeg_next::ChannelLayout::STEREO);
    fill_silence(&mut frame);
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
    r.push(&audio_frame(0, 1024), ffmpeg_next::Rational(1, 48_000))?;
    r.push(&audio_frame(1024, 1024), ffmpeg_next::Rational(1, 48_000))?;
    // 8 frames (~170ms) lost, then the stream resumes.
    r.push(
        &audio_frame(1024 * 10, 1024),
        ffmpeg_next::Rational(1, 48_000),
    )?;
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
    r.push(&audio_frame(0, 1024), ffmpeg_next::Rational(1, 48_000))?;
    r.push(
        &audio_frame(1024 + 100, 1024),
        ffmpeg_next::Rational(1, 48_000),
    )?; // ~2ms jitter
    r.push(
        &audio_frame(48_000 * 60, 1024),
        ffmpeg_next::Rational(1, 48_000),
    )?; // 60s jump
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

fn test_mp4_streams() -> Option<(AvInput, crate::stream::AvStream, crate::stream::AvStream)> {
    let path = crate::test_mp4_path();
    if !path.exists() {
        return None;
    }
    let input = AvInput::new(path.to_str()?, None, None).ok()?;
    let video = input.streams().values().find(|s| s.is_video())?.clone();
    let audio = input.streams().values().find(|s| s.is_audio())?.clone();
    Some((input, video, audio))
}

/// The source changing resolution mid-stream (camera profile switch) keeps
/// the transcode running: the scaler is rebuilt instead of rejecting every
/// later frame.
#[test]
fn test_encoder_survives_resolution_change() -> anyhow::Result<()> {
    use crate::frame::{RawFrame, RawVideoFrame};
    use ffmpeg_next::format::Pixel;
    let Some((_input, video, _)) = test_mp4_streams() else {
        return Ok(());
    };
    let settings = Settings {
        width: 160,
        height: 120,
        ..Settings::default()
    };
    let mut encoder = Encoder::new(&video, settings, None)?;
    let mut packets = 0;
    for (i, (w, h)) in [(320, 240), (320, 240), (640, 480), (640, 480)]
        .into_iter()
        .enumerate()
    {
        let mut f = ffmpeg_next::frame::Video::new(Pixel::YUV420P, w, h);
        f.set_pts(Some(i as i64 * 1024));
        encoder.send_frame(RawFrame::Video(RawVideoFrame::from(f)))?;
        while encoder.encoder_receive_packet()?.is_some() {
            packets += 1;
        }
    }
    encoder.send_eof()?;
    while encoder.encoder_receive_packet()?.is_some() {
        packets += 1;
    }
    assert_eq!(
        packets, 4,
        "every frame encoded across the resolution change"
    );
    Ok(())
}

/// The source changing sample rate / layout mid-stream keeps the audio
/// transcode running (the resampler is rebuilt).
#[test]
fn test_encoder_survives_audio_format_change() -> anyhow::Result<()> {
    use crate::encoder::AudioSettings;
    use crate::frame::{RawAudioFrame, RawFrame};
    use ffmpeg_next::{ChannelLayout, format::Sample, format::sample::Type};
    let Some((_input, _, audio)) = test_mp4_streams() else {
        return Ok(());
    };
    let mut encoder = Encoder::new_audio(&audio, AudioSettings::default(), None)?;
    let mut packets = 0;
    let mut pts = 0i64;
    for (rate, layout) in [
        (48_000, ChannelLayout::STEREO),
        (48_000, ChannelLayout::STEREO),
        (44_100, ChannelLayout::MONO),
        (44_100, ChannelLayout::MONO),
    ] {
        for _ in 0..10 {
            let mut f = ffmpeg_next::frame::Audio::new(Sample::F32(Type::Planar), 1024, layout);
            fill_silence(&mut f);
            f.set_rate(rate);
            f.set_pts(Some(pts));
            pts += 1024;
            encoder.send_frame(RawFrame::Audio(RawAudioFrame::from(f)))?;
            while encoder.encoder_receive_packet()?.is_some() {
                packets += 1;
            }
        }
    }
    encoder.send_eof()?;
    while encoder.encoder_receive_packet()?.is_some() {
        packets += 1;
    }
    assert!(
        packets > 30,
        "audio kept encoding across the change: {packets} packets"
    );
    Ok(())
}

#[test]
fn test_audio_format_change_preserves_gap() -> anyhow::Result<()> {
    use ffmpeg_next::{
        ChannelLayout,
        format::{Sample, sample::Type},
    };
    for (format, layout) in [
        (Sample::F32(Type::Planar), ChannelLayout::MONO),
        (Sample::F32(Type::Packed), ChannelLayout::STEREO),
    ] {
        let mut r = resampler()?;
        r.push(&audio_frame(0, 1024), ffmpeg_next::Rational(1, 48_000))?;
        r.push(&audio_frame(1024, 1024), ffmpeg_next::Rational(1, 48_000))?;
        let mut changed = ffmpeg_next::frame::Audio::new(format, 1024, layout);
        fill_silence(&mut changed);
        changed.set_rate(48_000);
        changed.set_pts(Some(10240));
        r.push(&changed, ffmpeg_next::Rational(1, 48_000))?;
        let mut out = r.drain()?;
        out.extend(r.flush()?);
        let last = out.last().unwrap();
        assert_eq!(last.pts().unwrap() + last.samples() as i64, 11264);
    }
    Ok(())
}

fn audio_stream(
    rate: u32,
    time_base: ffmpeg_next::Rational,
) -> anyhow::Result<crate::stream::AvStream> {
    crate::init()?;
    let input = AvInput::new(&format!("sine=sample_rate={rate}"), Some("lavfi"), None)?;
    let stream = input.streams().values().find(|s| s.is_audio()).unwrap();
    Ok(crate::stream::AvStream::new(
        stream.index(),
        stream.parameters().clone(),
        time_base,
        stream.rate(),
    ))
}

#[test]
fn test_audio_stream_time_base_is_converted_to_samples() -> anyhow::Result<()> {
    let stream = audio_stream(16_000, ffmpeg_next::Rational(1, 90_000))?;
    let mut encoder = Encoder::new_audio(&stream, super::AudioSettings::default(), None)?;
    for i in 0..17 {
        let mut frame = audio_frame(126_000 + i * 5760, 1024);
        frame.set_rate(16_000);
        encoder.send_frame(crate::frame::RawFrame::Audio(frame.into()))?;
        while encoder.encoder_receive_packet()?.is_some() {}
    }
    let r = encoder.audio_resampler.as_ref().unwrap();
    assert_eq!(
        r.next_pts,
        22_400 + 17 * 1024,
        "no false gaps; anchor at 1.4s"
    );
    encoder.send_eof()?;
    while encoder.encoder_receive_packet()?.is_some() {}
    Ok(())
}

#[test]
fn test_audio_gap_encodes_every_silence_and_source_frame() -> anyhow::Result<()> {
    let stream = audio_stream(48_000, ffmpeg_next::Rational(1, 48_000))?;
    let mut encoder = Encoder::new_audio(&stream, super::AudioSettings::default(), None)?;
    let mut pts = Vec::new();
    for p in [0, 1024, 10240] {
        encoder.send_frame(crate::frame::RawFrame::Audio(audio_frame(p, 1024).into()))?;
        while let Some(packet) = encoder.encoder_receive_packet()? {
            pts.push(packet.pts().unwrap());
        }
    }
    encoder.send_eof()?;
    while let Some(packet) = encoder.encoder_receive_packet()? {
        pts.push(packet.pts().unwrap());
    }
    // AAC emits one priming packet in addition to the eleven input chunks.
    assert_eq!(pts.len(), 12);
    assert!(
        pts.windows(2).all(|p| p[1] - p[0] == 1024),
        "missing packets: {pts:?}"
    );
    assert_eq!(pts.last(), Some(&10240));
    Ok(())
}
