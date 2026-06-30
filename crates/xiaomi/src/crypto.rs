//! Port of go2rtc `pkg/xiaomi/crypto/crypto.go`.
//!
//! A NaCl box (Curve25519 + HSalsa20) precomputed shared key is used as a
//! ChaCha20 key for the encrypted channel. Byte-compatible with the Go code:
//! `Encode` prefixes 8 random nonce bytes; the 12-byte ChaCha20 nonce is
//! `[0,0,0,0] ++ nonce8`.

use anyhow::{Context, Result};
use chacha20::ChaCha20;
use chacha20::cipher::{KeyIvInit, StreamCipher};
use dryoc::classic::crypto_box::{crypto_box_beforenm, crypto_box_keypair};
use rand::RngCore;

/// Generate a Curve25519 keypair. Returns `(public, private)`, 32 bytes each.
pub fn generate_key() -> (Vec<u8>, Vec<u8>) {
    let (public, secret) = crypto_box_keypair();
    (public.to_vec(), secret.to_vec())
}

/// Precompute the NaCl box shared key from the device's public key and our
/// private key (both hex-encoded), matching Go's `box.Precompute`.
pub fn calc_shared_key(device_public_hex: &str, client_private_hex: &str) -> Result<[u8; 32]> {
    let public = decode_hex32(device_public_hex).context("device public key")?;
    let private = decode_hex32(client_private_hex).context("client private key")?;
    Ok(crypto_box_beforenm(&public, &private))
}

fn decode_hex32(s: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(s)?;
    let len = bytes.len();
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("expected 32 bytes, got {len}"))
}

/// Encrypt `src` with `key32`, prefixing the 8 random nonce bytes (Go `Encode`).
/// Output layout: `[8-byte nonce][ciphertext]`.
pub fn encode(src: &[u8], key32: &[u8; 32]) -> Vec<u8> {
    let mut dst = vec![0u8; src.len() + 8];
    rand::thread_rng().fill_bytes(&mut dst[..8]);

    let mut nonce12 = [0u8; 12];
    nonce12[4..].copy_from_slice(&dst[..8]);

    dst[8..].copy_from_slice(src);
    chacha20_xor(key32, &nonce12, &mut dst[8..]);
    dst
}

/// Decrypt a buffer produced by [`encode`] (`[8-byte nonce][ciphertext]`).
pub fn decode(src: &[u8], key32: &[u8; 32]) -> Vec<u8> {
    decode_nonce(&src[8..], &src[..8], key32)
}

/// Decrypt `src` using an explicit 8-byte `nonce8` (Go `DecodeNonce`).
pub fn decode_nonce(src: &[u8], nonce8: &[u8], key32: &[u8; 32]) -> Vec<u8> {
    let mut nonce12 = [0u8; 12];
    nonce12[4..].copy_from_slice(nonce8);

    let mut dst = src.to_vec();
    chacha20_xor(key32, &nonce12, &mut dst);
    dst
}

fn chacha20_xor(key32: &[u8; 32], nonce12: &[u8; 12], buf: &mut [u8]) {
    let mut cipher =
        ChaCha20::new_from_slices(key32, nonce12).expect("valid 32-byte key and 12-byte nonce");
    cipher.apply_keystream(buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_round_trip() {
        let key = [7u8; 32];
        let plain = b"hello xiaomi camera frame";
        let enc = encode(plain, &key);
        assert_eq!(enc.len(), plain.len() + 8);
        assert_ne!(&enc[8..], &plain[..]); // ciphertext differs from plaintext
        assert_eq!(decode(&enc, &key), plain);
    }

    #[test]
    fn shared_key_is_symmetric() {
        let (pub_a, priv_a) = generate_key();
        let (pub_b, priv_b) = generate_key();
        let ka = calc_shared_key(&hex::encode(&pub_b), &hex::encode(&priv_a)).unwrap();
        let kb = calc_shared_key(&hex::encode(&pub_a), &hex::encode(&priv_b)).unwrap();
        assert_eq!(ka, kb); // Diffie-Hellman: both sides derive the same key
    }
}
