use super::*;

#[test]
fn decodes_little_endian_and_drops_an_odd_byte() {
    let bytes = [0x00, 0x40, 0x00, 0x80, 0xff, 0x7f, 0x12];
    assert_eq!(s16le_to_f32(&bytes), vec![0.5, -1.0, 32767.0 / 32768.0]);
}

#[test]
fn encodes_with_clamping() {
    let bytes = f32_to_s16le(&[0.0, 1.5, -2.0]);
    assert_eq!(bytes, vec![0x00, 0x00, 0xff, 0x7f, 0x01, 0x80]);
}

#[test]
fn round_trips_within_one_step() {
    let samples = [0.25, -0.75, 0.001];
    for (a, b) in samples.iter().zip(s16le_to_f32(&f32_to_s16le(&samples))) {
        assert!((a - b).abs() < 1.0 / 16384.0, "{a} vs {b}");
    }
}
