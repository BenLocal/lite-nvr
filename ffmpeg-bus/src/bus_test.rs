use std::path::{Path, PathBuf};

use futures::StreamExt;
use tokio::io::AsyncWriteExt as _;

use crate::bus::{Bus, InputConfig, OutputAvType, OutputConfig, OutputDest};
use crate::metadata::probe;

/// Path to scripts/test.mp4 relative to workspace root (parent of ffmpeg-bus). Works regardless of cwd.
fn test_mp4_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
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
        eprintln!("skip: {} not found", input_path.display());
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
        eprintln!("skip: {} not found", input_path.display());
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
        eprintln!("skip: {} not found", input_path.display());
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
