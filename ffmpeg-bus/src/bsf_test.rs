use super::*;

#[test]
fn test_is_annexb_packet_variants() {
    assert!(is_annexb_packet(&[0x00, 0x00, 0x00, 0x01, 0x67]));
    assert!(is_annexb_packet(&[0x00, 0x00, 0x01, 0x67]));
    assert!(!is_annexb_packet(&[0x01, 0x00, 0x00, 0x00]));
    assert!(!is_annexb_packet(&[0x00, 0x00]));
}

#[test]
fn test_convert_avcc_to_annexb_single_nal() {
    let avcc = [0, 0, 0, 4, 0x65, 0x88, 0x81, 0x00];
    let out = convert_avcc_to_annexb(&avcc);
    assert_eq!(
        &out[..],
        &[0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x81, 0x00][..]
    );
}

#[test]
fn test_convert_avcc_to_annexb_multiple_nal() {
    // NAL#1 len=3 (0x67,0x64,0x00), NAL#2 len=2 (0x68,0xee)
    let avcc = [0, 0, 0, 3, 0x67, 0x64, 0x00, 0, 0, 0, 2, 0x68, 0xee];
    let out = convert_avcc_to_annexb(&avcc);
    assert_eq!(
        &out[..],
        &[
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x00, 0x00, 0x00, 0x01, 0x68, 0xee,
        ][..]
    );
}

#[test]
fn test_convert_avcc_to_annexb_invalid_length_truncated() {
    // Declared NAL length 5, but only 3 bytes payload; conversion should stop safely.
    let avcc = [0, 0, 0, 5, 0xaa, 0xbb, 0xcc];
    let out = convert_avcc_to_annexb(&avcc);
    assert!(out.is_empty());
}

