//! GB/T 28181 integration for nvr: a global on-demand bridge over the gb28181
//! crate's GbServer, wired to ZLM's media hooks. See
//! docs/superpowers/specs/2026-07-01-gb28181-crate-design.md row 11.

pub mod config;
pub mod receiver;
pub mod stream_map;
