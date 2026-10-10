use std::sync::Arc;

use ffmpeg_bus::bus::{OutputConfig as FbOutputConfig, OutputDest as FbOutputDest};
use ffmpeg_bus::stream::AvStream;
use tokio::task::JoinHandle;

use super::{DemuxedSink, EncodeConfig, OutputConfig, OutputDest};
use ffmpeg_bus::bus::VideoRawFrameStream;

struct NullSink;

impl DemuxedSink for NullSink {
    fn start(&self, _av: AvStream, _stream: VideoRawFrameStream) -> JoinHandle<()> {
        unreachable!("mapping tests never start the sink")
    }
}

fn mapped(encode: Option<EncodeConfig>) -> FbOutputConfig {
    let dest = OutputDest::Demuxed {
        sink: Arc::new(NullSink),
    };
    let fb: Option<FbOutputConfig> = OutputConfig::new(dest, encode).into();
    fb.expect("demuxed output maps")
}

/// Without an encode a demuxed sink gets the input packets untouched.
#[test]
fn test_demuxed_without_encode_passes_through() {
    let fb = mapped(None);
    assert!(matches!(fb.dest, FbOutputDest::Demuxed));
    assert!(fb.encode.is_none());
}

/// With an encode a demuxed sink (e.g. ZLM) gets the encoder's packets,
/// instead of the encode config being silently dropped.
#[test]
fn test_demuxed_with_encode_uses_encoder_output() {
    let fb = mapped(Some(EncodeConfig {
        bitrate: Some(2_000_000),
        ..EncodeConfig::default()
    }));
    assert!(matches!(fb.dest, FbOutputDest::Encoded));
    assert_eq!(fb.encode.expect("encode kept").bitrate, Some(2_000_000));
}
