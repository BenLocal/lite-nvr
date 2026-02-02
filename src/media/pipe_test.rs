// ============================================================================
// Pipeline Tests
// ============================================================================

use std::sync::Arc;

use super::{Pipe, build_output, dest_name};
use crate::media::{
    stream::RawSinkSource,
    types::{EncodeConfig, InputConfig, OutputConfig, OutputDest, PipeConfig, VideoRawFrame},
};

// ------------------------------------------------------------------------
// PipeConfigBuilder Tests
// ------------------------------------------------------------------------

#[test]
fn test_builder_input_url() {
    let config = PipeConfig::builder()
        .input_url("rtsp://localhost:8554/stream")
        .add_remux_output("rtmp://localhost/live/test", "flv")
        .build();

    match &config.input {
        InputConfig::Network { url } => {
            assert_eq!(url, "rtsp://localhost:8554/stream");
        }
        _ => panic!("Expected Network input"),
    }

    let config = PipeConfig::builder()
        .input_file("test_video.mp4")
        .add_remux_output("rtmp://localhost/live/test", "flv")
        .build();

    match &config.input {
        InputConfig::File { path } => {
            assert_eq!(path, "test_video.mp4");
        }
        _ => panic!("Expected File input"),
    }
}

#[test]
fn test_builder_add_remux_output() {
    let config = PipeConfig::builder()
        .input_url("rtsp://localhost/stream")
        .add_remux_output("rtmp://localhost/live/test", "flv")
        .build();

    assert_eq!(config.outputs.len(), 1);
    match &config.outputs[0].dest {
        OutputDest::Network { url, format } => {
            assert_eq!(url, "rtmp://localhost/live/test");
            assert_eq!(format, "flv");
        }
        _ => panic!("Expected Network output"),
    }
    assert!(config.outputs[0].encode.is_none());
}

#[test]
fn test_builder_add_rtsp_output_with_encode() {
    let encode_config = EncodeConfig {
        codec: "h264".to_string(),
        width: Some(1280),
        height: Some(720),
        bitrate: Some(2_000_000),
        preset: Some("fast".to_string()),
        pixel_format: Some("yuv420p".to_string()),
    };

    let config = PipeConfig::builder()
        .input_url("rtsp://localhost/stream")
        .add_rtsp_output("rtsp://localhost:8554/out", encode_config)
        .build();

    assert_eq!(config.outputs.len(), 1);
    match &config.outputs[0].dest {
        OutputDest::Network { url, format } => {
            assert_eq!(url, "rtsp://localhost:8554/out");
            assert_eq!(format, "rtsp");
        }
        _ => panic!("Expected Network output"),
    }

    let encode = config.outputs[0].encode.as_ref().unwrap();
    assert_eq!(encode.codec, "h264");
    assert_eq!(encode.width, Some(1280));
    assert_eq!(encode.height, Some(720));
    assert_eq!(encode.bitrate, Some(2_000_000));
    assert_eq!(encode.preset, Some("fast".to_string()));
}

#[test]
fn test_builder_add_raw_frame_output() {
    let sink = Arc::new(RawSinkSource::new());
    let config = PipeConfig::builder()
        .input_url("rtsp://localhost/stream")
        .add_raw_frame_output(sink)
        .build();

    assert_eq!(config.outputs.len(), 1);
    match &config.outputs[0].dest {
        OutputDest::RawFrame { .. } => {}
        _ => panic!("Expected RawFrame output"),
    }
}

#[test]
fn test_builder_add_raw_packet_output() {
    let sink = Arc::new(RawSinkSource::new());
    let encode_config = EncodeConfig::default();

    let config = PipeConfig::builder()
        .input_url("rtsp://localhost/stream")
        .add_raw_packet_output(sink, encode_config)
        .build();

    assert_eq!(config.outputs.len(), 1);
    match &config.outputs[0].dest {
        OutputDest::RawPacket { .. } => {}
        _ => panic!("Expected RawPacket output"),
    }
    assert!(config.outputs[0].encode.is_some());
}

#[test]
fn test_builder_multiple_outputs() {
    let raw_sink = Arc::new(RawSinkSource::new());
    let packet_sink = Arc::new(RawSinkSource::new());

    let config = PipeConfig::builder()
        .input_url("rtsp://localhost/stream")
        .add_remux_output("rtmp://localhost/live/1", "flv")
        .add_remux_output("rtmp://localhost/live/2", "flv")
        .add_raw_frame_output(raw_sink)
        .add_raw_packet_output(packet_sink, EncodeConfig::default())
        .build();

    assert_eq!(config.outputs.len(), 4);
}

#[test]
#[should_panic(expected = "input is required")]
fn test_builder_missing_input_panics() {
    let _config = PipeConfig::builder()
        .add_remux_output("rtmp://localhost/live/test", "flv")
        .build();
}

// ------------------------------------------------------------------------
// EncodeConfig Tests
// ------------------------------------------------------------------------

#[test]
fn test_encode_config_default() {
    let config = EncodeConfig::default();

    assert_eq!(config.codec, "h264");
    assert!(config.width.is_none());
    assert!(config.height.is_none());
    assert!(config.bitrate.is_none());
    assert!(config.preset.is_none());
    assert!(config.pixel_format.is_none());
}

#[test]
fn test_encode_config_equality() {
    let config1 = EncodeConfig {
        codec: "h264".to_string(),
        width: Some(1920),
        height: Some(1080),
        bitrate: Some(4_000_000),
        preset: Some("medium".to_string()),
        pixel_format: Some("yuv420p".to_string()),
    };

    let config2 = EncodeConfig {
        codec: "h264".to_string(),
        width: Some(1920),
        height: Some(1080),
        bitrate: Some(4_000_000),
        preset: Some("medium".to_string()),
        pixel_format: Some("yuv420p".to_string()),
    };

    let config3 = EncodeConfig {
        codec: "hevc".to_string(),
        ..config1.clone()
    };

    assert_eq!(config1, config2);
    assert_ne!(config1, config3);
}

#[test]
fn test_encode_config_hash() {
    use std::collections::HashSet;

    let config1 = EncodeConfig {
        codec: "h264".to_string(),
        width: Some(1920),
        height: Some(1080),
        bitrate: None,
        preset: None,
        pixel_format: None,
    };

    let config2 = EncodeConfig {
        codec: "h264".to_string(),
        width: Some(1920),
        height: Some(1080),
        bitrate: None,
        preset: None,
        pixel_format: None,
    };

    let config3 = EncodeConfig {
        codec: "hevc".to_string(),
        width: Some(1920),
        height: Some(1080),
        bitrate: None,
        preset: None,
        pixel_format: None,
    };

    let mut set = HashSet::new();
    set.insert(config1.clone());

    assert!(set.contains(&config2));
    assert!(!set.contains(&config3));
}

// ------------------------------------------------------------------------
// dest_name Tests
// ------------------------------------------------------------------------

#[test]
fn test_dest_name_network() {
    let dest = OutputDest::Network {
        url: "rtmp://localhost/live/test".to_string(),
        format: "flv".to_string(),
    };
    assert_eq!(dest_name(&dest), "rtmp://localhost/live/test");
}

#[test]
fn test_dest_name_raw_frame() {
    let sink = Arc::new(RawSinkSource::new());
    let dest = OutputDest::RawFrame { sink };
    assert_eq!(dest_name(&dest), "RawFrame");
}

#[test]
fn test_dest_name_raw_packet() {
    let sink = Arc::new(RawSinkSource::new());
    let dest = OutputDest::RawPacket { sink };
    assert_eq!(dest_name(&dest), "RawPacket");
}

// ------------------------------------------------------------------------
// Pipe Tests
// ------------------------------------------------------------------------

#[test]
fn test_pipe_new() {
    let config = PipeConfig::builder()
        .input_url("rtsp://localhost/stream")
        .add_remux_output("rtmp://localhost/live/test", "flv")
        .build();

    let pipe = Pipe::new(config);
    assert!(!pipe.is_started());
}

#[test]
fn test_pipe_cancel() {
    let config = PipeConfig::builder()
        .input_url("rtsp://localhost/stream")
        .add_remux_output("rtmp://localhost/live/test", "flv")
        .build();

    let pipe = Pipe::new(config);
    assert!(!pipe.is_cancelled());

    pipe.cancel();
    assert!(pipe.is_cancelled());
}

// ------------------------------------------------------------------------
// build_output Tests
// ------------------------------------------------------------------------

#[test]
fn test_build_output_network_remux() {
    let config = OutputConfig {
        dest: OutputDest::Network {
            url: "rtmp://localhost/live/test".to_string(),
            format: "flv".to_string(),
        },
        encode: None,
    };

    let output = build_output(&config);
    assert!(output.is_some());
}

#[test]
fn test_build_output_network_with_encode() {
    let config = OutputConfig {
        dest: OutputDest::Network {
            url: "rtmp://localhost/live/test".to_string(),
            format: "flv".to_string(),
        },
        encode: Some(EncodeConfig {
            codec: "h264".to_string(),
            width: Some(1280),
            height: Some(720),
            bitrate: Some(2_000_000),
            preset: Some("fast".to_string()),
            pixel_format: None,
        }),
    };

    let output = build_output(&config);
    assert!(output.is_some());
}

#[test]
fn test_build_output_raw_frame() {
    let sink = Arc::new(RawSinkSource::new());
    let config = OutputConfig {
        dest: OutputDest::RawFrame { sink },
        encode: None,
    };

    let output = build_output(&config);
    assert!(output.is_some());
}

#[test]
fn test_build_output_raw_packet() {
    let sink = Arc::new(RawSinkSource::new());
    let config = OutputConfig {
        dest: OutputDest::RawPacket { sink },
        encode: Some(EncodeConfig::default()),
    };

    let output = build_output(&config);
    assert!(output.is_some());
}

#[test]
fn test_build_output_raw_packet_format_h264() {
    let sink = Arc::new(RawSinkSource::new());
    let config = OutputConfig {
        dest: OutputDest::RawPacket { sink },
        encode: Some(EncodeConfig {
            codec: "h264".to_string(),
            ..Default::default()
        }),
    };

    let output = build_output(&config);
    assert!(output.is_some());
}

#[test]
fn test_build_output_raw_packet_format_hevc() {
    let sink = Arc::new(RawSinkSource::new());
    let config = OutputConfig {
        dest: OutputDest::RawPacket { sink },
        encode: Some(EncodeConfig {
            codec: "hevc".to_string(),
            ..Default::default()
        }),
    };

    let output = build_output(&config);
    assert!(output.is_some());
}

// ------------------------------------------------------------------------
// RawSinkSource Tests
// ------------------------------------------------------------------------

#[test]
fn test_raw_sink_source_creation() {
    let sink = RawSinkSource::new();
    assert!(sink.writer.capacity() > 0);
}

#[test]
fn test_raw_sink_source_with_capacity() {
    let sink = RawSinkSource::with_capacity(64);
    assert_eq!(sink.writer.capacity(), 64);
}

#[tokio::test]
async fn test_raw_sink_source_send_receive() {
    let sink = Arc::new(RawSinkSource::new());
    let test_data = vec![1u8, 2, 3, 4, 5];

    // Create a VideoRawFrame
    let frame = VideoRawFrame::new(test_data.clone(), 640, 480, 0, 0, 0, true, 0);

    // Send data
    sink.writer.send(frame).await.unwrap();

    // Receive data
    use futures::StreamExt;
    let mut stream = RawSinkSource::as_stream(sink);
    let received = stream.next().await.unwrap();

    assert_eq!(received.data(), test_data.as_slice());
}

// ------------------------------------------------------------------------
// Integration Tests (require actual FFmpeg and media files)
// ------------------------------------------------------------------------

#[tokio::test]
#[ignore = "Requires actual RTSP server"]
async fn test_pipe_start_with_rtsp_input() {
    let config = PipeConfig::builder()
        .input_url("rtsp://localhost:8554/test")
        .add_remux_output("rtmp://localhost:1935/live/out", "flv")
        .build();

    let pipe = Arc::new(Pipe::new(config));
    let pipe_clone = pipe.clone();

    // Start pipe in background
    let handle = tokio::spawn(async move {
        pipe_clone.start().await;
    });

    // Wait a bit then cancel
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    pipe.cancel();

    handle.await.unwrap();
}

#[tokio::test]
#[ignore = "Requires actual media file"]
async fn test_pipe_raw_frame_output() {
    let sink = Arc::new(RawSinkSource::new());
    let sink_clone = sink.clone();

    let config = PipeConfig::builder()
        .input_url("test_video.mp4")
        .add_raw_frame_output(sink)
        .build();

    let pipe = Arc::new(Pipe::new(config));
    let pipe_clone = pipe.clone();

    // Start pipe in background
    let handle = tokio::spawn(async move {
        pipe_clone.start().await;
    });

    // Try to receive some frames
    use futures::StreamExt;
    let mut stream = RawSinkSource::as_stream(sink_clone);
    let mut frame_count = 0;

    while let Some(_frame) = stream.next().await {
        frame_count += 1;
        if frame_count >= 10 {
            break;
        }
    }

    pipe.cancel();
    handle.await.unwrap();

    assert!(frame_count > 0, "Should have received at least one frame");
}
