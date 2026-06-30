//! Native Rust port of go2rtc's Xiaomi camera support (miss / TUTK CS2 path).
//!
//! Ported phase by phase from `github.com/AlexxIT/go2rtc` `pkg/xiaomi`:
//! - [`crypto`] — Curve25519 (NaCl box) key exchange + ChaCha20 channel cipher.
//! - [`cloud`] — Xiaomi account auth + miio-signed cloud API.
//! - [`device`] — device discovery + miss/CS2 connection resolution.
//! - (next) TUTK CS2 transport, MTP producer.

pub mod cloud;
pub mod crypto;
pub mod device;
