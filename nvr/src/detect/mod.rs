//! Real-time object detection for live pipes: taps decoded video, samples,
//! fans out to N models, and serves the latest per-frame comparison over REST.

pub mod convert;
