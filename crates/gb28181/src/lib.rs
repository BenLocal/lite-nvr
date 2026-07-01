//! GB/T 28181 signaling core (media-agnostic). See
//! docs/superpowers/specs/2026-07-01-gb28181-crate-design.md.

pub mod auth;
pub mod encoding;
pub mod error;
pub mod gbcode;
pub mod manscdp;
pub mod registrar;
pub mod sdp;
pub mod types;

pub use error::GbError;
