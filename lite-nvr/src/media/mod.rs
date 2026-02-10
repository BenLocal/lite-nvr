//! Optimized Pipeline Architecture: Input produces two data types + shared Encoder for same config
//!
//! Data Flow:
//! ```text
//!                                 ┌─► RawPacket broadcast ─► [Remux outputs] (no re-encoding)
//!                                 │
//! Input (Demux) ──► decode? ──────┤
//!                                 │
//!                                 └─► DecodedFrame broadcast ─► [EncodeTask per config]
//!                                                                      │
//!                                                          ┌───────────┴───────────┐
//!                                                          ▼                       ▼
//!                                                   EncodedPacket             EncodedPacket
//!                                                          │                       │
//!                                                    [Mux outputs]           [Mux outputs]
//! ```
//!
//! Optimizations:
//! 1. Outputs that don't need re-encoding use RawPacket directly (remux)
//! 2. Outputs with the same encoding config share a single Encoder
//! 3. DecodedFrame can be consumed by raw frame sinks

#[allow(dead_code)]
pub mod pipe;
pub mod stream;
pub mod types;
