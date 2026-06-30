//! Native Rust port of go2rtc's Xiaomi camera support (miss / TUTK CS2 path).
//!
//! Ported phase by phase from `github.com/AlexxIT/go2rtc` `pkg/xiaomi`:
//! - [`crypto`] — Curve25519 (NaCl box) key exchange + ChaCha20 channel cipher.
//! - (next) cloud auth, device resolution, TUTK CS2 transport, MTP producer.

pub mod crypto;
