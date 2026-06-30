//! Native Rust port of go2rtc's Xiaomi camera support (miss / TUTK CS2 path).
//!
//! Ported phase by phase from `github.com/AlexxIT/go2rtc` `pkg/xiaomi`:
//! - [`crypto`] — Curve25519 (NaCl box) key exchange + ChaCha20 channel cipher.
//! - [`cloud`] — Xiaomi account auth + miio-signed cloud API.
//! - [`device`] — device discovery + miss/CS2 connection resolution.
//! - [`cs2`] — TUTK CS2 P2P transport (UDP/TCP).
//! - (next) miss client + MTP producer, lite-nvr integration.

pub mod cloud;
pub mod crypto;
pub mod cs2;
pub mod device;
