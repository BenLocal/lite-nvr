//! 16-bit little-endian PCM, the wire format for `/chat` audio both ways.

/// Decode s16le bytes to `f32` samples; a trailing odd byte is dropped.
pub(super) fn s16le_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
        .collect()
}

/// Encode `f32` samples (clamped to [-1, 1]) as s16le bytes.
pub(super) fn f32_to_s16le(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        // In range after the clamp.
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16).to_le_bytes())
        .collect()
}

#[cfg(test)]
#[path = "pcm_test.rs"]
mod pcm_test;
