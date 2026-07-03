use std::path::{Path, PathBuf};

use futures::StreamExt;
use tokio::io::AsyncWriteExt as _;

use crate::bus::{Bus, EncodeConfig, InputConfig, OutputAvType, OutputConfig, OutputDest};
use crate::encoder::{AudioSettings, Encoder, Settings};
use crate::input::AvInput;
use crate::metadata::probe;

/// Path to scripts/test.mp4 at the workspace root (crates/ffmpeg-bus/../..). Works regardless of cwd.
fn test_mp4_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join("scripts")
        .join("test.mp4")
}

/// Requires scripts/test.mp4 (~5s, 10fps).
#[tokio::test]
async fn test_mux_h264() -> anyhow::Result<()> {
    let file_name = "output.h264";
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
    let file_name = "output.aac";
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
            path: "output.mp4".to_string(),
        },
    );
    let _stream = bus.add_output(output_config).await?;

    // Source is ~5s @ 10fps; wait for mux to finish (read + write) then verify
    tokio::time::sleep(std::time::Duration::from_secs(8)).await;
    verify_output_mp4("output.mp4", Some(5.0), Some(10)).await?;
    Ok(())
}

/// Requires scripts/test.mp4. Transcodes the video to a smaller resolution and
/// muxes it to a file, exercising decode -> scale -> encode -> mux. Verifies the
/// output is a valid MP4 with a video stream.
#[tokio::test]
async fn test_transcode_video_to_file() -> anyhow::Result<()> {
    let file_name = "output_transcode.mp4";
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

/// Transcodes audio (copying video), forcing a resample (44100->48000), a
/// channel change (mono->stereo), and FIFO reframing to the AAC frame size.
/// Verifies both streams land in the MP4, the audio is really re-encoded to the
/// requested params, and audio/video stay time-aligned end to end (A/V sync).
#[tokio::test]
async fn test_transcode_audio_av_sync() -> anyhow::Result<()> {
    let file_name = "output_transcode_audio.mp4";
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

    let file_name = "output_rawvideo_test.h264";
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

    let output_path = "output_encode.aac";
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

    let output_path = "output_va.mp4";
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
