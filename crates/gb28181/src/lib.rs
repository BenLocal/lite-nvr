//! GB/T 28181 signaling core (media-agnostic). See
//! docs/superpowers/specs/2026-07-01-gb28181-crate-design.md.

pub mod auth;
pub mod client;
pub mod encoding;
pub mod endpoint;
pub mod error;
pub mod event;
pub mod gbcode;
pub mod manscdp;
pub mod registrar;
pub mod sdp;
pub mod server;
pub mod types;

pub use auth::AuthConfig;
pub use client::{ClientMediaHandle, GbClient, GbClientConfig, InviteNegotiation};
pub use error::GbError;
pub use event::GbEvent;
pub use gbcode::SsrcKind;
pub use manscdp::{Catalog, CatalogItem};
pub use server::{GbServer, GbServerConfig, MediaSession};
pub use types::{MediaSpec, RegisteredDevice, StreamType, Transport};
