//! Reusable, ZLM-agnostic media pipeline core.
//!
//! Wraps `ffmpeg-bus` into a [`Pipe`] driven by [`PipeConfig`], and forwards
//! each output to a destination. Raw/demuxed passthrough outputs go to a
//! caller-provided sink ([`RawSinkSource`] for frames/packets, or a
//! [`DemuxedSink`] implementation such as `media-pipe-zlm`'s `ZlmSink`).

pub mod pipe;
pub mod stream;
pub mod types;

pub use pipe::{Pipe, dest_name};
pub use stream::RawSinkSource;
pub use types::{
    DemuxedSink, EncodeConfig, InputConfig, OutputConfig, OutputDest, PipeConfig, VideoRawFrame,
};
