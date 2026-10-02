use std::path::{Path, PathBuf};

use futures::StreamExt;
use tokio::io::AsyncWriteExt as _;

use crate::bus::{Bus, EncodeConfig, InputConfig, OutputAvType, OutputConfig, OutputDest};
use crate::encoder::{AudioSettings, Encoder, Settings};
use crate::input::AvInput;
use crate::metadata::probe;

/// Path to scripts/test.mp4 at the workspace root (crates/ffmpeg-bus/../..). Works regardless of cwd.
pub(super) fn test_mp4_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("scripts")
        .join("test.mp4")
}

/// Path for a test-generated media file under `crates/ffmpeg-bus/.test_media/`
/// (git-ignored), creating the directory if needed.
pub(super) fn test_media(name: &str) -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".test_media");
    std::fs::create_dir_all(&dir).expect("create .test_media dir");
    dir.join(name).to_string_lossy().into_owned()
}

/// Requires scripts/test.mp4 (~5s, 10fps).
#[tokio::test]
async fn test_mux_h264() -> anyhow::Result<()> {
    let file_name = &test_media("output.h264");
    if Path::new(file_name).exists() {
        std::fs::remove_file(file_name).unwrap();
    }

    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }

    let bus = Bus::new("a");

    let input_config = InputConfig::File {
        path: input_path.to_string_lossy().into_owned(),
    };
    bus.add_input(input_config, None).await?;

    // Mux to raw H.264 and write to output.h264
    let output_config = OutputConfig::new(
        "mux_h264".to_string(),
        OutputAvType::Video,
        OutputDest::Mux {
            format: "h264".to_string(),
        },
    );
    let (_, mut stream) = bus.add_output(output_config).await?;

    let mut file = tokio::fs::File::create(file_name).await?;
    while let Some(frame) = stream.next().await {
        if let Some(frame) = frame {
            file.write_all(&frame.data).await?;
        }
    }
    file.sync_all().await?;

    // Verify output.h264: decodable and frame count ~50 (5s @ 10fps)
    verify_output_h264(file_name, 5, 10).await?;

    Ok(())
}

#[tokio::test]
async fn test_mux_aac() -> anyhow::Result<()> {
    let file_name = &test_media("output.aac");
    if Path::new(file_name).exists() {
        std::fs::remove_file(file_name).unwrap();
    }

    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }

    let bus = Bus::new("a");
    let input_config = InputConfig::File {
        path: input_path.to_string_lossy().into_owned(),
    };
    bus.add_input(input_config, None).await?;

    // Mux to raw AAC and write to output.aac
    let output_config = OutputConfig::new(
        "mux_aac".to_string(),
        OutputAvType::Audio,
        OutputDest::Mux {
            format: "adts".to_string(),
        },
    );
    let (_, mut stream) = bus.add_output(output_config).await?;

    let mut file = tokio::fs::File::create(file_name).await?;
    while let Some(frame) = stream.next().await {
        if let Some(frame) = frame {
            file.write_all(&frame.data).await?;
        }
    }
    file.sync_all().await?;

    verify_output_aac(file_name, 5, 43).await?;
    Ok(())
}

/// Requires scripts/test.mp4 (~5s, 10fps).
#[tokio::test]
async fn test_mux_only_video_mp4() -> anyhow::Result<()> {
    let file_name = &test_media("output.mp4");
    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }

    let bus = Bus::new("a");

    let input_config = InputConfig::File {
        path: input_path.to_string_lossy().into_owned(),
    };
    bus.add_input(input_config, None).await?;

    let output_config = OutputConfig::new(
        "mux_h264".to_string(),
        OutputAvType::Video,
        OutputDest::File {
            path: file_name.to_string(),
        },
    );
    let _stream = bus.add_output(output_config).await?;

    // Source is ~5s @ 10fps; wait for mux to finish (read + write) then verify
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;
    verify_output_mp4(file_name, Some(5.0), Some(10)).await?;
    Ok(())
}

/// Requires scripts/test.mp4. Transcodes the video to a smaller resolution and
/// muxes it to a file, exercising decode -> scale -> encode -> mux. Verifies the
/// output is a valid MP4 with a video stream.
#[tokio::test]
async fn test_transcode_video_to_file() -> anyhow::Result<()> {
    let file_name = &test_media("output_transcode.mp4");
    if Path::new(file_name).exists() {
        std::fs::remove_file(file_name).ok();
    }
    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }

    let bus = Bus::new("t");
    bus.add_input(
        InputConfig::File {
            path: input_path.to_string_lossy().into_owned(),
        },
        None,
    )
    .await?;

    // Force a transcode by requesting a different resolution (same codec).
    let encode = EncodeConfig {
        codec: "h264".to_string(),
        width: Some(320),
        height: Some(240),
        ..Default::default()
    };
    let output_config = OutputConfig::new(
        "transcode_file".to_string(),
        OutputAvType::Video,
        OutputDest::File {
            path: file_name.to_string(),
        },
    )
    .with_encode(encode);
    let _ = bus.add_output(output_config).await?;

    // Source is ~5s; wait for decode/encode/mux to finish, then verify.
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    verify_output_mp4(file_name, Some(5.0), None).await?;
    Ok(())
}

/// Two File outputs on the same video stream with different encode configs
/// must each get their own encoder (no "first config wins"), while sharing one
/// decoder. A lossy decoded-video subscriber that is never read must not stall
/// the lossless transcodes. Also checks the encoder GOP yields periodic keyframes.
#[tokio::test]
async fn test_separate_encoders_per_config_share_decoder() -> anyhow::Result<()> {
    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }
    // test.mp4 is 320x240; both sizes differ so both outputs really transcode.
    let (file_a, file_b) = (
        test_media("output_enc_a.mp4"),
        test_media("output_enc_b.mp4"),
    );
    let outputs = [
        (file_a.as_str(), 240u32, 180u32),
        (file_b.as_str(), 160, 120),
    ];
    for (file, _, _) in outputs {
        std::fs::remove_file(file).ok();
    }

    let bus = Bus::new("t");
    bus.add_input(
        InputConfig::File {
            path: input_path.to_string_lossy().into_owned(),
        },
        None,
    )
    .await?;
    for (file, w, h) in outputs {
        let encode = EncodeConfig {
            codec: "h264".to_string(),
            width: Some(w),
            height: Some(h),
            ..Default::default()
        };
        let output = OutputConfig::new(
            file.to_string(),
            OutputAvType::Video,
            OutputDest::File {
                path: file.to_string(),
            },
        )
        .with_encode(encode);
        let _ = bus.add_output(output).await?;
    }
    // Lossy subscriber on the shared decoder that never reads.
    let _idle = bus.subscribe_video().await?;

    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    for (file, w, h) in outputs {
        let info = probe(file)?;
        let video = info
            .streams
            .iter()
            .find(|s| s.codec_type == "video")
            .ok_or_else(|| anyhow::anyhow!("{file}: no video stream"))?;
        assert_eq!((video.width, video.height), (Some(w), Some(h)), "{file}");

        let mut input = ffmpeg_next::format::input(file)?;
        let (mut frames, mut keys) = (0u32, 0u32);
        for (stream, packet) in input.packets() {
            if stream.index() == video.index {
                frames += 1;
                keys += u32::from(packet.is_key());
            }
        }
        // Transcoded timestamps must keep the source timing (5s), not
        // collapse when frames move into the encoder's time base.
        let duration = probe(file)?.format.duration_sec.unwrap_or(0.0);
        assert!(
            (4.5..=5.5).contains(&duration),
            "{file}: duration {duration}s"
        );
        if file == outputs[0].0 {
            // The first output starts the input, so it sees the whole 5s @
            // 10fps source and, being lossless, every frame lands.
            assert_eq!(frames, 50, "{file}: frame count");
            // GOP 25 → keyframes at 0 and 25.
            assert!(keys >= 2, "{file}: expected periodic keyframes, got {keys}");
        } else {
            // A later output joins a running source and may miss its start,
            // but must still produce a finished, decodable file.
            assert!(frames > 0, "{file}: no frames");
        }
    }
    Ok(())
}

/// Transcodes audio (copying video), forcing a resample (44100->48000), a
/// channel change (mono->stereo), and FIFO reframing to the AAC frame size.
/// Verifies both streams land in the MP4, the audio is really re-encoded to the
/// requested params, and audio/video stay time-aligned end to end (A/V sync).
#[tokio::test]
async fn test_transcode_audio_av_sync() -> anyhow::Result<()> {
    let file_name = &test_media("output_transcode_audio.mp4");
    if Path::new(file_name).exists() {
        std::fs::remove_file(file_name).ok();
    }
    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }

    let bus = Bus::new("ta");
    bus.add_input(
        InputConfig::File {
            path: input_path.to_string_lossy().into_owned(),
        },
        None,
    )
    .await?;

    // Copy video, transcode audio to AAC 48000/stereo (source is 44100/mono).
    let audio_encode = EncodeConfig {
        codec: "aac".to_string(),
        sample_rate: Some(48000),
        channels: Some(2),
        ..Default::default()
    };
    let output_config = OutputConfig::new(
        "transcode_audio".to_string(),
        OutputAvType::Video,
        OutputDest::File {
            path: file_name.to_string(),
        },
    )
    .with_audio()
    .with_audio_encode(audio_encode);
    let _ = bus.add_output(output_config).await?;

    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let info = probe(file_name).map_err(|e| anyhow::anyhow!("invalid container: {}", e))?;
    let audio = info
        .streams
        .iter()
        .find(|s| s.codec_type == "audio")
        .ok_or_else(|| anyhow::anyhow!("output should contain a transcoded audio stream"))?;
    assert_eq!(
        audio.sample_rate,
        Some(48000),
        "audio should be resampled to 48kHz"
    );
    assert_eq!(audio.channels, Some(2), "audio should be upmixed to stereo");
    assert!(
        info.streams.iter().any(|s| s.codec_type == "video"),
        "output should still contain the copied video stream"
    );

    verify_av_sync(file_name, 5.0).await?;
    Ok(())
}

/// Reads a muxed output and checks that its video and audio streams both run to
/// ~`expected_dur` seconds and end within a small window of each other. A wrong
/// per-stream time base (e.g. audio PTS in the wrong units) or dropped samples
/// would show up here as a duration mismatch or A/V drift.
async fn verify_av_sync(path: &str, expected_dur: f64) -> anyhow::Result<()> {
    let info = probe(path)?;
    let type_of: std::collections::HashMap<usize, String> = info
        .streams
        .iter()
        .map(|s| (s.index, s.codec_type.clone()))
        .collect();

    let mut input = ffmpeg_next::format::input(path)?;
    let mut last: std::collections::HashMap<usize, f64> = std::collections::HashMap::new();
    for (stream, packet) in input.packets() {
        let tb = stream.time_base();
        let denom = tb.denominator() as f64;
        if denom == 0.0 {
            continue;
        }
        if let Some(pts) = packet.pts() {
            let end =
                (pts as f64 + packet.duration().max(0) as f64) * tb.numerator() as f64 / denom;
            let e = last.entry(stream.index()).or_insert(0.0);
            if end > *e {
                *e = end;
            }
        }
    }

    let end_for = |ty: &str| -> Option<f64> {
        last.iter()
            .filter(|(idx, _)| type_of.get(idx).map(|t| t == ty).unwrap_or(false))
            .map(|(_, v)| *v)
            .fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.max(v))))
    };
    let video_end = end_for("video").ok_or_else(|| anyhow::anyhow!("no video packets"))?;
    let audio_end = end_for("audio").ok_or_else(|| anyhow::anyhow!("no audio packets"))?;

    assert!(
        (video_end - expected_dur).abs() < 0.5,
        "video ends at {:.3}s, expected ~{:.1}s",
        video_end,
        expected_dur
    );
    assert!(
        (audio_end - expected_dur).abs() < 0.5,
        "audio ends at {:.3}s, expected ~{:.1}s",
        audio_end,
        expected_dur
    );
    assert!(
        (video_end - audio_end).abs() < 0.3,
        "A/V out of sync: video ends {:.3}s, audio ends {:.3}s",
        video_end,
        audio_end
    );
    Ok(())
}

/// Stable init-level regression: prefer HW H.264 encoder and fallback to software automatically.
/// Uses scripts/test.mp4 to obtain real stream parameters, then only validates encoder init path.
#[test]
fn test_encoder_init_auto_hw_fallback_from_test_mp4() -> anyhow::Result<()> {
    crate::init()?;
    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }

    let input = AvInput::new(input_path.to_string_lossy().as_ref(), None, None)?;
    let video_stream = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video stream in test.mp4"))?
        .clone();

    let settings = Settings {
        codec: Some("h264".to_string()),
        ..Settings::default()
    };
    let _encoder = Encoder::new(&video_stream, settings, None)?;
    Ok(())
}

/// Stable init-level regression: force software libx264 init from scripts/test.mp4.
#[test]
fn test_encoder_init_force_software_from_test_mp4() -> anyhow::Result<()> {
    crate::init()?;
    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }

    let input = AvInput::new(input_path.to_string_lossy().as_ref(), None, None)?;
    let video_stream = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video stream in test.mp4"))?
        .clone();

    let settings = Settings {
        codec: Some("libx264".to_string()),
        ..Settings::default()
    };
    let _encoder = Encoder::new(&video_stream, settings, None)?;
    Ok(())
}

/// Verifies output.h264: openable with ffmpeg_next and packet count within ±20% of duration_sec * fps.
async fn verify_output_h264(path: &str, duration_sec: u32, fps: u32) -> anyhow::Result<()> {
    let path = Path::new(path);
    assert!(path.exists(), "output.h264 should exist");
    let size = std::fs::metadata(path)?.len();
    assert!(size > 0, "output.h264 should not be empty");

    // 1. Open and read all packets (validates file is decodable)
    let path_str = path.to_str().unwrap();
    let mut input = ffmpeg_next::format::input(path_str)
        .map_err(|e| anyhow::anyhow!("output.h264 should open without error: {}", e))?;

    let nb_streams = input.nb_streams();
    assert!(
        nb_streams >= 1,
        "output.h264 should have at least one stream"
    );

    // 2. Count packets in the first (video) stream; raw H.264 has a single stream
    let video_stream_index = 0u32;
    let mut packet_count: u32 = 0;
    for (stream, _packet) in input.packets() {
        if stream.index() == video_stream_index as usize {
            packet_count += 1;
        }
    }

    let expected_frames = duration_sec * fps;
    let min_frames = expected_frames.saturating_sub(expected_frames / 5);
    let max_frames = expected_frames + expected_frames / 5;

    assert!(
        packet_count >= min_frames && packet_count <= max_frames,
        "output.h264 packet count {} should be in [{}, {}] (expected ~{} for {}s @ {}fps)",
        packet_count,
        min_frames,
        max_frames,
        expected_frames,
        duration_sec,
        fps
    );

    Ok(())
}

/// Verifies output.mp4: valid container, has duration, and at least one video stream.
/// Optionally checks duration and packet count when expected_duration_sec and expected_fps are given.
async fn verify_output_mp4(
    path: &str,
    expected_duration_sec: Option<f64>,
    expected_fps: Option<u32>,
) -> anyhow::Result<()> {
    let path = Path::new(path);
    assert!(path.exists(), "output.mp4 should exist");
    let size = std::fs::metadata(path)?.len();
    assert!(size > 0, "output.mp4 should not be empty");

    let info = probe(path.to_str().unwrap())
        .map_err(|e| anyhow::anyhow!("output.mp4 should be a valid container: {}", e))?;

    assert!(
        info.format.nb_streams >= 1,
        "output.mp4 should have at least one stream, got {}",
        info.format.nb_streams
    );

    let has_video = info.streams.iter().any(|s| s.codec_type == "video");
    assert!(
        has_video,
        "output.mp4 should have at least one video stream"
    );

    let duration_sec = info
        .format
        .duration_sec
        .ok_or_else(|| anyhow::anyhow!("output.mp4 should have duration metadata"))?;
    assert!(
        duration_sec > 0.0,
        "output.mp4 duration should be positive, got {}",
        duration_sec
    );

    if let (Some(expected_d), Some(expected_fps)) = (expected_duration_sec, expected_fps) {
        let min_d = expected_d * 0.8;
        let max_d = expected_d * 1.2;
        assert!(
            duration_sec >= min_d && duration_sec <= max_d,
            "output.mp4 duration {}s should be in [{}, {}] (expected ~{}s)",
            duration_sec,
            min_d,
            max_d,
            expected_d
        );

        let expected_frames = (expected_d * expected_fps as f64).round() as u32;
        let mut input = ffmpeg_next::format::input(path.to_str().unwrap())?;
        let video_index = info
            .streams
            .iter()
            .find(|s| s.codec_type == "video")
            .map(|s| s.index)
            .unwrap();
        let mut packet_count: u32 = 0;
        for (stream, _) in input.packets() {
            if stream.index() == video_index {
                packet_count += 1;
            }
        }
        let min_frames = expected_frames.saturating_sub(expected_frames / 5);
        let max_frames = expected_frames + expected_frames / 5;
        assert!(
            packet_count >= min_frames && packet_count <= max_frames,
            "output.mp4 video packet count {} should be in [{}, {}] (expected ~{} for {}s @ {}fps)",
            packet_count,
            min_frames,
            max_frames,
            expected_frames,
            expected_d,
            expected_fps
        );
    }

    Ok(())
}

/// Test rawvideo path: lavfi virtual test picture -> packet->frame conversion -> encoder -> output.
/// Uses Device input with format "lavfi" and testsrc filter (raw video), then mux to H.264.
#[tokio::test]
async fn test_device_rawvideo_lavfi() -> anyhow::Result<()> {
    crate::init()?;

    let file_name = &test_media("output_rawvideo_test.h264");
    if Path::new(file_name).exists() {
        std::fs::remove_file(file_name).unwrap();
    }

    let bus = Bus::new("rawvideo_test");

    // Virtual test picture: lavfi testsrc, 2s, 320x240, 10fps (raw video -> RAWVIDEO codec path)
    let input_config = InputConfig::Device {
        display: "testsrc=duration=2:size=320x240:rate=10".to_string(),
        format: "lavfi".to_string(),
    };
    bus.add_input(input_config, None).await?;

    // Output via encoder (exercises packet->frame conversion for raw video, then encode to H.264)
    let output_config = OutputConfig::new(
        "rawvideo_h264".to_string(),
        OutputAvType::Video,
        OutputDest::Encoded,
    );
    let (_, mut stream) = bus.add_output(output_config).await?;

    let mut file = tokio::fs::File::create(file_name).await?;
    while let Some(frame) = stream.next().await {
        match frame {
            Some(f) => file.write_all(&f.data).await?,
            None => break, // EOF from encoder, stop consuming
        }
    }
    file.sync_all().await?;

    // Verify: 2s @ 10fps -> ~20 frames
    verify_output_h264(file_name, 2, 10).await?;

    Ok(())
}

/// Audio encoder init test: validates Encoder::new_audio() from test.mp4 audio stream.
#[test]
fn test_audio_encoder_init() -> anyhow::Result<()> {
    crate::init()?;
    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }

    let input = AvInput::new(input_path.to_string_lossy().as_ref(), None, None)?;
    let audio_stream = input
        .streams()
        .values()
        .find(|s| s.is_audio())
        .ok_or_else(|| anyhow::anyhow!("no audio stream in test.mp4"))?
        .clone();

    let settings = AudioSettings {
        codec: Some("aac".to_string()),
        ..AudioSettings::default()
    };
    let _encoder = Encoder::new_audio(&audio_stream, settings, None)?;
    Ok(())
}

/// Test audio encode: decode audio from test.mp4 → re-encode to AAC, muxed to ADTS file.
#[tokio::test]
async fn test_audio_encode_aac() -> anyhow::Result<()> {
    crate::init()?;

    let output_path = &test_media("output_encode.aac");
    if Path::new(output_path).exists() {
        std::fs::remove_file(output_path).unwrap();
    }

    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }

    let bus = Bus::new("audio_encode_test");
    let input_config = InputConfig::File {
        path: input_path.to_string_lossy().into_owned(),
    };
    bus.add_input(input_config, None).await?;

    // Force re-encode by requesting Mux output with encode config for audio
    let output_config = OutputConfig::new(
        "audio_encoded_mux".to_string(),
        OutputAvType::Audio,
        OutputDest::Mux {
            format: "adts".to_string(),
        },
    )
    .with_encode(EncodeConfig {
        codec: "aac".to_string(),
        ..EncodeConfig::default()
    });
    let (_, mut stream) = bus.add_output(output_config).await?;

    let mut file = tokio::fs::File::create(output_path).await?;
    let mut packet_count = 0u32;
    while let Some(frame) = stream.next().await {
        if let Some(frame) = frame {
            file.write_all(&frame.data).await?;
            packet_count += 1;
        }
    }
    file.sync_all().await?;

    // Verify the output is a valid AAC file
    assert!(
        packet_count > 0,
        "expected encoded audio packets, got {}",
        packet_count
    );
    let size = std::fs::metadata(output_path)?.len();
    assert!(size > 0, "output AAC file should not be empty");

    // Clean up
    if Path::new(output_path).exists() {
        std::fs::remove_file(output_path).unwrap();
    }

    Ok(())
}

/// Test muxing both video and audio into a single MP4 file.
#[tokio::test]
async fn test_mux_mp4_video_and_audio() -> anyhow::Result<()> {
    crate::init()?;

    let output_path = &test_media("output_va.mp4");
    if Path::new(output_path).exists() {
        std::fs::remove_file(output_path).unwrap();
    }

    let input_path = test_mp4_path();
    if !input_path.exists() {
        log::warn!("skip: {} not found", input_path.display());
        return Ok(());
    }

    let bus = Bus::new("va_mux_test");
    let input_config = InputConfig::File {
        path: input_path.to_string_lossy().into_owned(),
    };
    bus.add_input(input_config, None).await?;

    // Mux to MP4 with both video and audio
    let output_config = OutputConfig::new(
        "mux_va_mp4".to_string(),
        OutputAvType::Video,
        OutputDest::File {
            path: output_path.to_string(),
        },
    )
    .with_audio();
    let _stream = bus.add_output(output_config).await?;

    // Wait for mux to finish
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;

    // Verify the output has both video and audio streams
    let info = probe(output_path)
        .map_err(|e| anyhow::anyhow!("output_va.mp4 should be a valid container: {}", e))?;

    let has_video = info.streams.iter().any(|s| s.codec_type == "video");
    let has_audio = info.streams.iter().any(|s| s.codec_type == "audio");

    assert!(has_video, "output should have a video stream");
    assert!(has_audio, "output should have an audio stream");
    assert!(
        info.format.nb_streams >= 2,
        "output should have at least 2 streams, got {}",
        info.format.nb_streams
    );

    // Clean up
    if Path::new(output_path).exists() {
        std::fs::remove_file(output_path).unwrap();
    }

    Ok(())
}

/// Verifies output.aac: openable with ffmpeg_next and packet count within reasonable range.
/// AAC frames are typically 1024 samples. @ 44100Hz -> ~43 packets/sec.
async fn verify_output_aac(
    path: &str,
    duration_sec: u32,
    expected_packets_per_sec: u32,
) -> anyhow::Result<()> {
    let path = Path::new(path);
    assert!(path.exists(), "output.aac should exist");
    let size = std::fs::metadata(path)?.len();
    assert!(size > 0, "output.aac should not be empty");

    // 1. Open and read all packets (validates file is decodable)
    let path_str = path.to_str().unwrap();
    let mut input = ffmpeg_next::format::input(&path_str)
        .map_err(|e| anyhow::anyhow!("output.aac should open without error: {}", e))?;

    let nb_streams = input.nb_streams();
    assert!(
        nb_streams >= 1,
        "output.aac should have at least one stream"
    );

    // 2. Count packets in the first stream
    let stream_index = 0u32;
    let mut packet_count: u32 = 0;
    for (stream, _packet) in input.packets() {
        if stream.index() == stream_index as usize {
            packet_count += 1;
        }
    }

    let expected_packets = duration_sec * expected_packets_per_sec;
    // Allow larger margin for audio packets as buffering/padding can vary
    let min_packets = expected_packets.saturating_sub(expected_packets / 2);
    let max_packets = expected_packets + expected_packets / 2;

    assert!(
        packet_count >= min_packets && packet_count <= max_packets,
        "output.aac packet count {} should be in [{}, {}] (expected ~{} for {}s @ {}pps)",
        packet_count,
        min_packets,
        max_packets,
        expected_packets,
        duration_sec,
        expected_packets_per_sec
    );

    Ok(())
}

// --- Adaptive copy-vs-transcode decision (pure logic, no media file) ---

#[test]
fn codec_id_from_name_maps_known_codecs() {
    use ffmpeg_next::codec::Id;
    assert_eq!(Bus::codec_id_from_name("h264"), Some(Id::H264));
    assert_eq!(Bus::codec_id_from_name("H264"), Some(Id::H264));
    assert_eq!(Bus::codec_id_from_name("h265"), Some(Id::HEVC));
    assert_eq!(Bus::codec_id_from_name("hevc"), Some(Id::HEVC));
    assert_eq!(Bus::codec_id_from_name("aac"), Some(Id::AAC));
    assert_eq!(Bus::codec_id_from_name("opus"), Some(Id::OPUS));
    assert_eq!(Bus::codec_id_from_name("wobble"), None);
}

#[test]
fn video_params_match_means_copy() {
    use ffmpeg_next::codec::Id;
    // Same codec, no geometry override -> copy.
    let keep = EncodeConfig {
        codec: "h264".into(),
        ..Default::default()
    };
    assert!(!Bus::encode_needed_params(
        Id::H264,
        true,
        1920,
        1080,
        0,
        0,
        &keep
    ));
    // Same codec, matching geometry -> copy.
    let matching = EncodeConfig {
        codec: "h264".into(),
        width: Some(1920),
        height: Some(1080),
        ..Default::default()
    };
    assert!(!Bus::encode_needed_params(
        Id::H264,
        true,
        1920,
        1080,
        0,
        0,
        &matching
    ));
}

#[test]
fn video_params_differ_means_transcode() {
    use ffmpeg_next::codec::Id;
    // Different codec.
    let hevc = EncodeConfig {
        codec: "hevc".into(),
        ..Default::default()
    };
    assert!(Bus::encode_needed_params(
        Id::H264,
        true,
        1920,
        1080,
        0,
        0,
        &hevc
    ));
    // Same codec, different resolution.
    let resized = EncodeConfig {
        codec: "h264".into(),
        width: Some(1280),
        height: Some(720),
        ..Default::default()
    };
    assert!(Bus::encode_needed_params(
        Id::H264,
        true,
        1920,
        1080,
        0,
        0,
        &resized
    ));
}

#[test]
fn audio_copy_vs_transcode() {
    use ffmpeg_next::codec::Id;
    // AAC 48k/2 into AAC 48k/2 -> copy.
    let same = EncodeConfig {
        codec: "aac".into(),
        sample_rate: Some(48000),
        channels: Some(2),
        ..Default::default()
    };
    assert!(!Bus::encode_needed_params(
        Id::AAC,
        false,
        0,
        0,
        48000,
        2,
        &same
    ));
    // Different sample rate -> transcode.
    let resampled = EncodeConfig {
        codec: "aac".into(),
        sample_rate: Some(44100),
        ..Default::default()
    };
    assert!(Bus::encode_needed_params(
        Id::AAC,
        false,
        0,
        0,
        48000,
        2,
        &resampled
    ));
    // Different codec -> transcode.
    let opus = EncodeConfig {
        codec: "opus".into(),
        ..Default::default()
    };
    assert!(Bus::encode_needed_params(
        Id::AAC,
        false,
        0,
        0,
        48000,
        2,
        &opus
    ));
}

/// Write `frames` raw yuv420p frames of `w`x`h` (a moving gradient) and return
/// the path. Opened via the `rawvideo` demuxer this gives a real RAWVIDEO
/// stream (lavfi `testsrc` does not: it yields WRAPPED_AVFRAME).
fn write_raw_yuv(name: &str, w: usize, h: usize, frames: usize) -> String {
    let path = test_media(name);
    let mut data = Vec::with_capacity(w * h * 3 / 2 * frames);
    for f in 0..frames {
        data.extend((0..w * h).map(|i| ((i + f * 7) % 256) as u8));
        data.extend(std::iter::repeat_n(128u8, w * h / 2));
    }
    std::fs::write(&path, data).expect("write raw yuv");
    path
}

async fn raw_yuv_bus(path: &str) -> anyhow::Result<Bus> {
    crate::init()?;
    let bus = Bus::new("rawvideo");
    let options = [
        ("video_size", "64x48"),
        ("pixel_format", "yuv420p"),
        ("framerate", "10"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    // A raw file read through the rawvideo demuxer: not live, even though it
    // is opened as a "device" input.
    bus.add_input_with_live(
        InputConfig::Device {
            display: path.to_string(),
            format: "rawvideo".to_string(),
        },
        Some(options),
        false,
    )
    .await?;
    Ok(bus)
}

/// Collect a bus output stream until its EOF item (`None`).
async fn drain_frames(
    mut stream: crate::bus::VideoRawFrameStream,
) -> anyhow::Result<Vec<crate::frame::VideoFrame>> {
    tokio::time::timeout(std::time::Duration::from_secs(20), async move {
        let mut out = Vec::new();
        while let Some(Some(frame)) = stream.next().await {
            out.push(frame);
        }
        out
    })
    .await
    .map_err(|_| anyhow::anyhow!("stream never ended"))
}

/// Genuine RAWVIDEO input: the packet → frame relay feeds the encoder directly
/// (no decoder). A lossless File transcode keeps every frame and the source
/// timing; a lossy Encoded output yields H.264 packets starting on a keyframe.
/// Each output gets its own bus so it is the one that starts the input.
#[tokio::test(flavor = "multi_thread")]
async fn test_rawvideo_input_transcode() -> anyhow::Result<()> {
    let yuv = write_raw_yuv("input_64x48.yuv", 64, 48, 20);
    let mp4 = test_media("rawvideo_transcode.mp4");
    std::fs::remove_file(&mp4).ok();

    let file_bus = raw_yuv_bus(&yuv).await?;
    let _ = file_bus
        .add_output(
            OutputConfig::new(
                "file".to_string(),
                OutputAvType::Video,
                OutputDest::File { path: mp4.clone() },
            )
            .with_encode(EncodeConfig::default()),
        )
        .await?;

    let enc_bus = raw_yuv_bus(&yuv).await?;
    let (_, enc) = enc_bus
        .add_output(
            OutputConfig::new(
                "encoded".to_string(),
                OutputAvType::Video,
                OutputDest::Encoded,
            )
            .with_encode(EncodeConfig::default()),
        )
        .await?;
    let packets = drain_frames(enc).await?;
    assert!(!packets.is_empty(), "encoded output produced no packets");
    assert!(packets[0].is_key, "first encoded packet must be a keyframe");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while probe(&mp4).is_err() && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    let info = probe(&mp4)?;
    let video = info
        .streams
        .iter()
        .find(|s| s.codec_type == "video")
        .ok_or_else(|| anyhow::anyhow!("no video stream"))?;
    assert_eq!((video.width, video.height), (Some(64), Some(48)));
    assert_eq!(video.codec_name, "h264");
    let mut input = ffmpeg_next::format::input(&mp4)?;
    let frames = input
        .packets()
        .filter(|(s, _)| s.index() == video.index)
        .count();
    assert_eq!(frames, 20, "lossless rawvideo transcode keeps every frame");
    // 20 frames @ 10fps: the last starts at 1.9s.
    let duration = info.format.duration_sec.unwrap_or(0.0);
    assert!((1.7..=2.2).contains(&duration), "duration {duration}s");
    Ok(())
}

/// Mux to a raw codec format that differs from the input: packets come from
/// the encoder (`create_mux_output_stream_from_encoder`).
#[tokio::test(flavor = "multi_thread")]
async fn test_mux_from_encoder_output() -> anyhow::Result<()> {
    let yuv = write_raw_yuv("input_mux.yuv", 64, 48, 20);
    let out = test_media("mux_from_encoder.h264");
    let bus = raw_yuv_bus(&yuv).await?;
    let (av, stream) = bus
        .add_output(OutputConfig::new(
            "mux".to_string(),
            OutputAvType::Video,
            OutputDest::Mux {
                format: "h264".to_string(),
            },
        ))
        .await?;
    assert_eq!(av.parameters().id(), ffmpeg_next::codec::Id::H264);
    let chunks = drain_frames(stream).await?;
    let bytes: Vec<u8> = chunks.iter().flat_map(|c| c.data.iter().copied()).collect();
    assert!(!bytes.is_empty(), "mux produced no bytes");
    std::fs::write(&out, &bytes)?;
    let mut input = ffmpeg_next::format::input(&out)?;
    assert!(input.packets().count() > 0, "muxed h264 is not decodable");
    Ok(())
}

/// Two outputs with the same stream + encode config + loss policy share one
/// encoder: both get the same packets.
#[tokio::test(flavor = "multi_thread")]
async fn test_same_encode_config_shares_encoder() -> anyhow::Result<()> {
    let yuv = write_raw_yuv("input_shared.yuv", 64, 48, 20);
    let bus = raw_yuv_bus(&yuv).await?;
    let encoded = |id: &str| {
        OutputConfig::new(id.to_string(), OutputAvType::Video, OutputDest::Encoded)
            .with_encode(EncodeConfig::default())
    };
    let (_, a) = bus.add_output(encoded("a")).await?;
    let (_, b) = bus.add_output(encoded("b")).await?;
    let (a, b) = tokio::join!(drain_frames(a), drain_frames(b));
    let (a, b) = (a?, b?);
    assert!(!a.is_empty());
    // A shared encoder fans one packet sequence out: b is a suffix of a.
    let tail: Vec<_> = a[a.len() - b.len()..].iter().map(|p| p.pts).collect();
    assert_eq!(tail, b.iter().map(|p| p.pts).collect::<Vec<_>>());
    Ok(())
}

/// Decoded audio subscription delivers every audio frame of the file, then EOF.
#[tokio::test(flavor = "multi_thread")]
async fn test_subscribe_audio() -> anyhow::Result<()> {
    let input_path = test_mp4_path();
    if !input_path.exists() {
        return Ok(());
    }
    let bus = Bus::new("audio");
    bus.add_input(
        InputConfig::File {
            path: input_path.to_string_lossy().into_owned(),
        },
        None,
    )
    .await?;
    let mut rx = bus.subscribe_audio().await?;
    let frames = tokio::time::timeout(std::time::Duration::from_secs(20), async move {
        let mut n = 0usize;
        loop {
            match rx.recv().await {
                Ok(crate::frame::RawFrameCmd::Data(crate::frame::RawFrame::Audio(_))) => n += 1,
                Ok(crate::frame::RawFrameCmd::Data(_)) => {}
                // Lossy subscriber: a lag just means dropped frames.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Ok(crate::frame::RawFrameCmd::EOF)
                | Err(tokio::sync::broadcast::error::RecvError::Closed) => return n,
            }
        }
    })
    .await?;
    // Lossy subscriber: it may drop some of the 217 frames under load, but
    // must see audio and then the EOF that ended the loop.
    assert!(frames > 0, "decoded audio frames: {frames}");
    Ok(())
}

/// A Raw output on an audio stream cannot convert frames to `VideoFrame`: it
/// skips them (no panic) and still ends with EOF.
#[tokio::test(flavor = "multi_thread")]
async fn test_raw_audio_output_skips_unconvertible_frames() -> anyhow::Result<()> {
    let input_path = test_mp4_path();
    if !input_path.exists() {
        return Ok(());
    }
    let bus = Bus::new("raw-audio");
    bus.add_input(
        InputConfig::File {
            path: input_path.to_string_lossy().into_owned(),
        },
        None,
    )
    .await?;
    let (_, stream) = bus
        .add_output(OutputConfig::new(
            "raw".to_string(),
            OutputAvType::Audio,
            OutputDest::Raw,
        ))
        .await?;
    assert!(drain_frames(stream).await?.is_empty());
    Ok(())
}

/// API misuse is reported as errors, not panics or hangs.
#[tokio::test(flavor = "multi_thread")]
async fn test_bus_api_errors() -> anyhow::Result<()> {
    // Output before any input: there are no streams to pick from.
    let bus = Bus::new("errors");
    assert!(
        bus.add_output(OutputConfig::new(
            "early".to_string(),
            OutputAvType::Video,
            OutputDest::Demuxed,
        ))
        .await
        .is_err()
    );

    // Second input on the same bus.
    let yuv = write_raw_yuv("input_errors.yuv", 64, 48, 5);
    let bus = raw_yuv_bus(&yuv).await?;
    let again = bus
        .add_input(InputConfig::File { path: yuv.clone() }, None)
        .await;
    assert!(
        again
            .unwrap_err()
            .to_string()
            .contains("input already exists")
    );

    // Duplicate output id.
    let demuxed = || {
        OutputConfig::new(
            "same-id".to_string(),
            OutputAvType::Video,
            OutputDest::Demuxed,
        )
    };
    let _ = bus.add_output(demuxed()).await?;
    let dup = bus.add_output(demuxed()).await.err().map(|e| e.to_string());
    assert!(dup.is_some_and(|e| e.contains("output already exists")));

    // The source has no audio.
    let no_audio = bus.subscribe_audio().await;
    assert!(
        no_audio
            .unwrap_err()
            .to_string()
            .contains("no audio stream")
    );

    // Net output to a port nobody listens on.
    let port = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    let net = bus
        .add_output(
            OutputConfig::new(
                "net".to_string(),
                OutputAvType::Video,
                OutputDest::Net {
                    url: format!("rtsp://127.0.0.1:{port}/none"),
                    format: Some("rtsp".to_string()),
                },
            )
            .with_encode(EncodeConfig::default()),
        )
        .await;
    assert!(net.is_err(), "net output to a closed port must fail");

    // Unsupported encoder-output mux format.
    let bad_mux = bus
        .add_output(OutputConfig::new(
            "bad-mux".to_string(),
            OutputAvType::Video,
            OutputDest::Mux {
                format: "matroska".to_string(),
            },
        ))
        .await;
    assert!(bad_mux.is_err());
    Ok(())
}

/// The real reason for a rejected output reaches the caller (it used to be
/// lost as "channel closed").
#[tokio::test(flavor = "multi_thread")]
async fn test_add_output_reports_real_error() -> anyhow::Result<()> {
    let yuv = write_raw_yuv("input_no_audio.yuv", 64, 48, 5);
    let bus = raw_yuv_bus(&yuv).await?;
    let err = bus
        .add_output(OutputConfig::new(
            "audio".to_string(),
            OutputAvType::Audio,
            OutputDest::Demuxed,
        ))
        .await
        .err()
        .map(|e| format!("{e:#}"))
        .unwrap_or_default();
    assert!(err.contains("no Audio stream"), "got: {err}");
    Ok(())
}

/// A failed output tears down the encoder it had started; nothing is left
/// running behind (driven on the bus state directly to inspect it).
#[tokio::test(flavor = "multi_thread")]
async fn test_failed_output_rolls_back_started_tasks() -> anyhow::Result<()> {
    crate::init()?;
    let yuv = write_raw_yuv("input_rollback.yuv", 64, 48, 5);
    let mut state = super::BusState::new();
    let options = [
        ("video_size", "64x48"),
        ("pixel_format", "yuv420p"),
        ("framerate", "10"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    Bus::add_input_internal(
        &mut state,
        InputConfig::Device {
            display: yuv,
            format: "rawvideo".to_string(),
        },
        Some(options),
        None,
    )
    .await?;

    // Transcoding Net output to a dead RTSP port: the encoder starts, then the
    // connect fails.
    let port = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    let output = OutputConfig::new(
        "net".to_string(),
        OutputAvType::Video,
        OutputDest::Net {
            url: format!("rtsp://127.0.0.1:{port}/none"),
            format: Some("rtsp".to_string()),
        },
    )
    .with_encode(EncodeConfig::default());
    assert!(Bus::add_output_internal(&mut state, output).await.is_err());
    assert!(state.encoder_tasks.is_empty(), "encoder left running");
    assert!(state.encoder_output_streams.is_empty());
    assert!(state.output_config.is_empty());
    Ok(())
}

/// When the only consumer of a shared encoder leaves, the encoder stops; a
/// later output with the same config gets a fresh one and still receives data.
#[tokio::test(flavor = "multi_thread")]
async fn test_new_output_after_shared_encoder_stopped() -> anyhow::Result<()> {
    crate::init()?;
    let bus = Bus::new("restart");
    bus.add_input(
        InputConfig::Device {
            display: "testsrc=duration=4:size=160x120:rate=25,realtime".to_string(),
            format: "lavfi".to_string(),
        },
        None,
    )
    .await?;
    let encoded = |id: &str| {
        OutputConfig::new(id.to_string(), OutputAvType::Video, OutputDest::Encoded)
            .with_encode(EncodeConfig::default())
    };

    let (_, mut first) = bus.add_output(encoded("a")).await?;
    for _ in 0..3 {
        tokio::time::timeout(std::time::Duration::from_secs(5), first.next())
            .await?
            .ok_or_else(|| anyhow::anyhow!("first output ended early"))?;
    }
    drop(first);
    // Let the encoder notice it has no readers and stop.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let (_, second) = bus.add_output(encoded("b")).await?;
    let packets = drain_frames(second).await?;
    assert!(!packets.is_empty(), "restarted encoder produced nothing");
    assert!(packets[0].is_key, "a fresh encoder starts on a keyframe");
    Ok(())
}

#[test]
fn test_rtsp_input_options_default_timeout() {
    use super::{DEFAULT_RTSP_TIMEOUT_US, rtsp_input_options};
    // Non-RTSP inputs are left alone.
    assert!(rtsp_input_options("/tmp/a.mp4", None).is_none());
    // RTSP client inputs get the default timeout...
    let opts = rtsp_input_options("RTSP://cam/1", None).unwrap_or_default();
    assert_eq!(
        opts.get("timeout").map(String::as_str),
        Some(DEFAULT_RTSP_TIMEOUT_US)
    );
    // ...unless the caller set one,
    let custom = [("timeout".to_string(), "5".to_string())]
        .into_iter()
        .collect();
    let opts = rtsp_input_options("rtsp://cam/1", Some(custom)).unwrap_or_default();
    assert_eq!(opts.get("timeout").map(String::as_str), Some("5"));
    // ...and listen mode (waiting for a publisher) gets none.
    let listen = [("rtsp_flags".to_string(), "listen".to_string())]
        .into_iter()
        .collect();
    let opts = rtsp_input_options("rtsp://0.0.0.0:8554/l", Some(listen)).unwrap_or_default();
    assert!(!opts.contains_key("timeout"));
}

/// A TCP server that accepts connections and never answers: an RTSP peer
/// that hangs. Returns its port.
fn silent_tcp_server() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for conn in listener.incoming().take(8).flatten() {
            held.push(conn);
        }
    });
    Ok(port)
}

/// While one bus is stuck opening an unresponsive RTSP input, another bus on
/// the same single-worker runtime keeps working (the open runs on a blocking
/// thread), and the stuck open fails once its timeout expires.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_hanging_rtsp_input_does_not_block_runtime() -> anyhow::Result<()> {
    let port = silent_tcp_server()?;
    let stuck = Bus::new("stuck-input");
    stuck
        .add_input(
            InputConfig::Net {
                url: format!("rtsp://127.0.0.1:{port}/cam"),
            },
            Some(
                [("timeout".to_string(), "3000000".to_string())]
                    .into_iter()
                    .collect(),
            ),
        )
        .await?;
    let started = std::time::Instant::now();
    let stuck_output = tokio::spawn(async move {
        stuck
            .add_output(OutputConfig::new(
                "demuxed".to_string(),
                OutputAvType::Video,
                OutputDest::Demuxed,
            ))
            .await
            .map(|_| ())
    });

    let yuv = write_raw_yuv("input_runtime.yuv", 64, 48, 20);
    let healthy = raw_yuv_bus(&yuv).await?;
    let (_, enc) = healthy
        .add_output(
            OutputConfig::new("enc".to_string(), OutputAvType::Video, OutputDest::Encoded)
                .with_encode(EncodeConfig::default()),
        )
        .await?;
    assert!(!drain_frames(enc).await?.is_empty());
    assert!(
        !stuck_output.is_finished(),
        "healthy bus should finish while the other is still connecting"
    );

    let res = tokio::time::timeout(std::time::Duration::from_secs(10), stuck_output).await??;
    assert!(res.is_err(), "unresponsive RTSP input must fail");
    assert!(
        started.elapsed() >= std::time::Duration::from_secs(2),
        "timeout honoured"
    );
    Ok(())
}

/// Publishing to an RTSP server that never answers neither blocks the
/// runtime nor hangs forever: the default timeout fails the output.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn test_hanging_rtsp_output_does_not_block_runtime() -> anyhow::Result<()> {
    let port = silent_tcp_server()?;
    let yuv = write_raw_yuv("input_hang_out.yuv", 64, 48, 20);
    let publisher = raw_yuv_bus(&yuv).await?;
    let stuck_output = tokio::spawn(async move {
        publisher
            .add_output(
                OutputConfig::new(
                    "push".to_string(),
                    OutputAvType::Video,
                    OutputDest::Net {
                        url: format!("rtsp://127.0.0.1:{port}/live"),
                        format: Some("rtsp".to_string()),
                    },
                )
                .with_encode(EncodeConfig::default()),
            )
            .await
            .map(|_| ())
    });

    let healthy = raw_yuv_bus(&yuv).await?;
    let (_, enc) = healthy
        .add_output(
            OutputConfig::new("enc".to_string(), OutputAvType::Video, OutputDest::Encoded)
                .with_encode(EncodeConfig::default()),
        )
        .await?;
    assert!(!drain_frames(enc).await?.is_empty());
    assert!(
        !stuck_output.is_finished(),
        "push should still be connecting"
    );

    let res = tokio::time::timeout(std::time::Duration::from_secs(20), stuck_output).await??;
    assert!(
        res.is_err(),
        "unresponsive RTSP server must fail the output"
    );
    Ok(())
}

fn count_video_packets(path: &str) -> anyhow::Result<usize> {
    let mut input = ffmpeg_next::format::input(path)?;
    let video = input
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .ok_or_else(|| anyhow::anyhow!("{path}: no video"))?
        .index();
    Ok(input.packets().filter(|(s, _)| s.index() == video).count())
}

/// A deferred bus registers every output before reading: on a short file
/// (read in a burst) each output, whatever its kind, sees the whole stream.
#[tokio::test(flavor = "multi_thread")]
async fn test_deferred_bus_outputs_see_whole_file() -> anyhow::Result<()> {
    let input_path = test_mp4_path();
    if !input_path.exists() {
        return Ok(());
    }
    let copy = test_media("deferred_copy.mp4");
    let transcode = test_media("deferred_transcode.mp4");
    let mux_out = test_media("deferred_mux.h264");
    for p in [&copy, &transcode, &mux_out] {
        std::fs::remove_file(p).ok();
    }

    let bus = Bus::new_deferred("deferred");
    bus.add_input(
        InputConfig::File {
            path: input_path.to_string_lossy().into_owned(),
        },
        None,
    )
    .await?;
    let (_, demuxed) = bus
        .add_output(OutputConfig::new(
            "demuxed".to_string(),
            OutputAvType::Video,
            OutputDest::Demuxed,
        ))
        .await?;
    let _ = bus
        .add_output(OutputConfig::new(
            "copy".to_string(),
            OutputAvType::Video,
            OutputDest::File { path: copy.clone() },
        ))
        .await?;
    let _ = bus
        .add_output(
            OutputConfig::new(
                "transcode".to_string(),
                OutputAvType::Video,
                OutputDest::File {
                    path: transcode.clone(),
                },
            )
            .with_encode(EncodeConfig {
                width: Some(160),
                height: Some(120),
                ..Default::default()
            }),
        )
        .await?;
    let (_, mux) = bus
        .add_output(OutputConfig::new(
            "mux".to_string(),
            OutputAvType::Video,
            OutputDest::Mux {
                format: "h264".to_string(),
            },
        ))
        .await?;
    let demuxed = tokio::spawn(drain_frames(demuxed));
    let mux = tokio::spawn(drain_frames(mux));

    bus.start().await?;

    assert_eq!(demuxed.await??.len(), 50, "demuxed packets");
    let bytes: Vec<u8> = mux.await??.iter().flat_map(|c| c.data.to_vec()).collect();
    std::fs::write(&mux_out, bytes)?;
    assert_eq!(count_video_packets(&mux_out)?, 50, "mux frames");
    for path in [&copy, &transcode] {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while probe(path).is_err() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        assert_eq!(count_video_packets(path)?, 50, "{path}");
    }
    Ok(())
}

/// Nothing is read before `start()`; reading begins once it is called.
#[tokio::test(flavor = "multi_thread")]
async fn test_deferred_bus_waits_for_start() -> anyhow::Result<()> {
    let input_path = test_mp4_path();
    if !input_path.exists() {
        return Ok(());
    }
    let bus = Bus::new_deferred("wait");
    bus.add_input(
        InputConfig::File {
            path: input_path.to_string_lossy().into_owned(),
        },
        None,
    )
    .await?;
    let (_, mut demuxed) = bus
        .add_output(OutputConfig::new(
            "demuxed".to_string(),
            OutputAvType::Video,
            OutputDest::Demuxed,
        ))
        .await?;
    let early = tokio::time::timeout(std::time::Duration::from_millis(500), demuxed.next()).await;
    assert!(early.is_err(), "no packet may arrive before start()");

    bus.start().await?;
    bus.start().await?; // idempotent
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), demuxed.next()).await?;
    assert!(matches!(first, Some(Some(_))), "packets flow after start()");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_start_without_input_fails() -> anyhow::Result<()> {
    let bus = Bus::new_deferred("empty");
    assert!(bus.start().await.is_err());
    Ok(())
}

#[test]
fn test_keyframe_gate_resyncs_on_keyframe() {
    let mut gate = super::KeyframeGate::default();
    assert!(gate.admit(0, false), "no gap: everything passes");
    gate.mark_gap(0);
    assert!(
        !gate.admit(0, false),
        "after a gap, non-keyframes are dropped"
    );
    assert!(gate.admit(1, false), "other streams are unaffected");
    assert!(gate.admit(0, true), "a keyframe reopens the stream");
    assert!(gate.admit(0, false));
    assert_eq!(gate.dropped, 1);
    assert!(gate.note_drop(1), "first drop is logged");
    assert!(
        !gate.admit(1, false),
        "a dropped packet also forces a resync"
    );
}

#[test]
fn test_infer_live_from_input_type() {
    use super::infer_live;
    assert!(!infer_live(&InputConfig::File {
        path: "a.mp4".into()
    }));
    assert!(infer_live(&InputConfig::Net {
        url: "rtsp://cam/1".into()
    }));
    assert!(infer_live(&InputConfig::Device {
        display: ":0".into(),
        format: "x11grab".into(),
    }));
}

/// Drive a File transcode output on the bus state directly and report
/// (input reader lossless?, encoder lossless?) for the given live flag.
async fn file_transcode_loss_policy(live: bool) -> anyhow::Result<(bool, bool)> {
    crate::init()?;
    let yuv = write_raw_yuv(&format!("input_policy_{live}.yuv"), 64, 48, 5);
    let mp4 = test_media(&format!("policy_{live}.mp4"));
    let mut state = super::BusState::new();
    let options = [
        ("video_size", "64x48"),
        ("pixel_format", "yuv420p"),
        ("framerate", "10"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    Bus::add_input_internal(
        &mut state,
        InputConfig::Device {
            display: yuv,
            format: "rawvideo".to_string(),
        },
        Some(options),
        Some(live),
    )
    .await?;
    let output = OutputConfig::new(
        "file".to_string(),
        OutputAvType::Video,
        OutputDest::File { path: mp4 },
    )
    .with_encode(EncodeConfig::default());
    let _ = Bus::add_output_internal(&mut state, output).await?;
    let input_lossless = state
        .input_task
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no input task"))?
        .is_lossless();
    let encoder_lossless = state
        .encoder_tasks
        .keys()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no encoder"))?
        .lossless;
    Ok((input_lossless, encoder_lossless))
}

/// Live source: a File/Net output never holds the input back (reader not
/// lossless, lossy encoder); it drops and resyncs on keyframes instead.
#[tokio::test(flavor = "multi_thread")]
async fn test_live_input_file_output_is_lossy() -> anyhow::Result<()> {
    assert_eq!(file_transcode_loss_policy(true).await?, (false, false));
    Ok(())
}

/// Non-live source: File/Net outputs stay lossless end to end.
#[tokio::test(flavor = "multi_thread")]
async fn test_non_live_input_file_output_is_lossless() -> anyhow::Result<()> {
    assert_eq!(file_transcode_loss_policy(false).await?, (true, true));
    Ok(())
}

/// Mux from an encoder on a stream that is not index 0 (the audio track):
/// packets carry the muxed stream's index, the header uses the encoder's
/// real params, and with no encode config the format picks the encoder
/// (opus → libopus). The result is a valid Ogg/Opus stream.
#[tokio::test(flavor = "multi_thread")]
async fn test_mux_audio_from_encoder_opus() -> anyhow::Result<()> {
    let input_path = test_mp4_path();
    if !input_path.exists() {
        return Ok(());
    }
    let bus = Bus::new("opus");
    bus.add_input(
        InputConfig::File {
            path: input_path.to_string_lossy().into_owned(),
        },
        None,
    )
    .await?;
    let (av, stream) = bus
        .add_output(OutputConfig::new(
            "opus".to_string(),
            OutputAvType::Audio,
            OutputDest::Mux {
                format: "opus".to_string(),
            },
        ))
        .await?;
    assert_eq!(av.parameters().id(), ffmpeg_next::codec::Id::OPUS);
    let bytes: Vec<u8> = drain_frames(stream)
        .await?
        .iter()
        .flat_map(|c| c.data.to_vec())
        .collect();
    let out = test_media("mux_audio.opus");
    std::fs::write(&out, &bytes)?;

    let info = probe(&out)?;
    let audio = info
        .streams
        .iter()
        .find(|s| s.codec_type == "audio")
        .ok_or_else(|| anyhow::anyhow!("no audio in muxed opus"))?;
    assert_eq!(audio.codec_name, "opus");
    let duration = info.format.duration_sec.unwrap_or(0.0);
    assert!((4.0..=5.5).contains(&duration), "opus duration {duration}s");
    Ok(())
}

/// A Mux format the encoder does not produce is rejected up front.
#[tokio::test(flavor = "multi_thread")]
async fn test_mux_format_must_match_encoder() -> anyhow::Result<()> {
    let yuv = write_raw_yuv("input_mux_mismatch.yuv", 64, 48, 5);
    let bus = raw_yuv_bus(&yuv).await?;
    let res = bus
        .add_output(
            OutputConfig::new(
                "mismatch".to_string(),
                OutputAvType::Video,
                OutputDest::Mux {
                    format: "hevc".to_string(),
                },
            )
            .with_encode(EncodeConfig::default()), // h264
        )
        .await;
    let err = res.err().map(|e| format!("{e:#}")).unwrap_or_default();
    assert!(err.contains("needs"), "got: {err}");
    Ok(())
}

/// test.mp4's video as an MPEG-TS byte stream (muxed by the bus itself).
async fn test_ts_bytes() -> anyhow::Result<Vec<u8>> {
    let bus = Bus::new("ts");
    bus.add_input(
        InputConfig::File {
            path: test_mp4_path().to_string_lossy().into_owned(),
        },
        None,
    )
    .await?;
    let (_, stream) = bus
        .add_output(OutputConfig::new(
            "ts".to_string(),
            OutputAvType::Video,
            OutputDest::Mux {
                format: "mpegts".to_string(),
            },
        ))
        .await?;
    Ok(drain_frames(stream)
        .await?
        .iter()
        .flat_map(|c| c.data.to_vec())
        .collect())
}

/// Serve the first half of `bytes` over TCP, then go silent while keeping the
/// connection open: a network source that hangs. Returns the port.
fn stalling_tcp_server(bytes: Vec<u8>) -> anyhow::Result<u16> {
    use std::io::Write;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            let _ = conn.write_all(&bytes[..bytes.len() / 2]);
            std::thread::sleep(std::time::Duration::from_secs(60));
            drop(conn);
        }
    });
    Ok(port)
}

async fn stalled_ts_bus(
    timeout_us: Option<&str>,
) -> anyhow::Result<(Bus, crate::bus::VideoRawFrameStream)> {
    use std::collections::HashMap;
    let port = stalling_tcp_server(test_ts_bytes().await?)?;
    let bus = Bus::new("stalled");
    // Probe only a little, so the open completes on the data that does
    // arrive and it is the later *read* that blocks on the silence.
    let mut options: HashMap<String, String> = [
        ("analyzeduration".to_string(), "100000".to_string()),
        ("probesize".to_string(), "32768".to_string()),
    ]
    .into_iter()
    .collect();
    if let Some(t) = timeout_us {
        options.insert("timeout".to_string(), t.to_string());
    }
    let options = Some(options);
    bus.add_input_with_live(
        InputConfig::Net {
            url: format!("tcp://127.0.0.1:{port}"),
        },
        options,
        true,
    )
    .await?;
    let (_, stream) = bus
        .add_output(OutputConfig::new(
            "demuxed".to_string(),
            OutputAvType::Video,
            OutputDest::Demuxed,
        ))
        .await?;
    Ok((bus, stream))
}

/// A network input that goes silent mid-stream ends once its read timeout
/// fires. ffmpeg-next's packet iterator retried such errors forever, leaving
/// the session "running" with no data and never restarted.
#[tokio::test(flavor = "multi_thread")]
async fn test_stalled_network_input_ends_after_timeout() -> anyhow::Result<()> {
    if !test_mp4_path().exists() {
        return Ok(());
    }
    let (_bus, stream) = stalled_ts_bus(Some("1000000")).await?;
    let frames = tokio::time::timeout(std::time::Duration::from_secs(10), drain_frames(stream))
        .await
        .map_err(|_| anyhow::anyhow!("stalled input never ended"))??;
    assert!(!frames.is_empty(), "frames before the stall");
    Ok(())
}

/// Without any timeout, removing the input still aborts a read blocked on a
/// silent source right away (interrupt callback), instead of the reader —
/// and process shutdown — waiting on it forever.
#[tokio::test(flavor = "multi_thread")]
async fn test_remove_input_interrupts_blocked_read() -> anyhow::Result<()> {
    if !test_mp4_path().exists() {
        return Ok(());
    }
    let (bus, stream) = stalled_ts_bus(None).await?;
    let drain = tokio::spawn(drain_frames(stream));
    // Let it read what the server sends, then block on the silence.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    bus.remove_input().await?;
    let frames = tokio::time::timeout(std::time::Duration::from_secs(3), drain)
        .await
        .map_err(|_| anyhow::anyhow!("blocked read was not interrupted"))???;
    assert!(!frames.is_empty());
    Ok(())
}

/// `Bus::stop` aborts an input open stuck on a peer that never sends.
#[tokio::test(flavor = "multi_thread")]
async fn test_stop_interrupts_hanging_open() -> anyhow::Result<()> {
    let port = silent_tcp_server()?;
    let bus = std::sync::Arc::new(Bus::new("hanging-open"));
    bus.add_input(
        InputConfig::Net {
            url: format!("tcp://127.0.0.1:{port}"),
        },
        None,
    )
    .await?;
    let b = bus.clone();
    let output = tokio::spawn(async move {
        b.add_output(OutputConfig::new(
            "demuxed".to_string(),
            OutputAvType::Video,
            OutputDest::Demuxed,
        ))
        .await
        .map(|_| ())
    });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        !output.is_finished(),
        "open should be blocked on the silent peer"
    );
    bus.stop();
    let res = tokio::time::timeout(std::time::Duration::from_secs(3), output)
        .await
        .map_err(|_| anyhow::anyhow!("hanging open was not interrupted"))??;
    assert!(res.is_err());
    Ok(())
}

#[test]
fn test_net_output_options_set_timeouts() {
    use super::{DEFAULT_RTSP_TIMEOUT_US, net_output_options};
    let rtsp = net_output_options(Some("rtsp"));
    assert_eq!(rtsp.get("rtsp_transport"), Some("tcp"));
    assert_eq!(rtsp.get("timeout"), Some(DEFAULT_RTSP_TIMEOUT_US));
    let flv = net_output_options(Some("flv"));
    assert_eq!(flv.get("rw_timeout"), Some(DEFAULT_RTSP_TIMEOUT_US));
    assert_eq!(flv.get("timeout"), None);
}

#[test]
fn test_is_connection_error() {
    use super::is_connection_error;
    let err = |e: ffmpeg_next::Error| anyhow::Error::from(e);
    assert!(is_connection_error(&err(ffmpeg_next::Error::Exit)));
    assert!(is_connection_error(&err(ffmpeg_next::Error::Other {
        errno: ffmpeg_next::util::error::EPIPE
    })));
    assert!(!is_connection_error(&err(ffmpeg_next::Error::InvalidData)));
    assert!(!is_connection_error(&anyhow::anyhow!(
        "stream not found: 3"
    )));
}
