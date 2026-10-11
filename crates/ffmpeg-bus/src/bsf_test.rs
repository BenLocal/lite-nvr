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

#[test]
fn test_avcc_parameter_sets() {
    // version 1, profile/compat/level, lengthSize, 1 SPS (2 bytes), 1 PPS (1 byte)
    let avcc = [
        1, 0x64, 0, 0x1f, 0xff, 0xe1, 0, 2, 0x67, 0xaa, 1, 0, 1, 0x68,
    ];
    let out = avcc_parameter_sets(&avcc).expect("parameter sets");
    assert_eq!(out.as_ref(), &[0, 0, 0, 1, 0x67, 0xaa, 0, 0, 0, 1, 0x68]);
}

#[test]
fn test_avcc_parameter_sets_rejects_malformed() {
    assert!(avcc_parameter_sets(&[]).is_none());
    assert!(
        avcc_parameter_sets(&[0, 1, 2, 3, 4, 5]).is_none(),
        "not version 1"
    );
    // SPS length runs past the end: no panic, just None.
    assert!(avcc_parameter_sets(&[1, 0x64, 0, 0x1f, 0xff, 0xe1, 0, 9, 0x67]).is_none());
}

#[test]
fn test_hvcc_parameter_sets_keeps_vps_sps_pps() {
    let mut hvcc = vec![1u8];
    hvcc.extend([0u8; 21]); // rest of the 22-byte header
    hvcc.push(4); // numArrays
    for (ty, nal) in [(32u8, 0x40u8), (33, 0x42), (34, 0x44), (39, 0x4e)] {
        hvcc.extend([ty, 0, 1, 0, 1, nal]);
    }
    let out = hvcc_parameter_sets(&hvcc).expect("parameter sets");
    assert_eq!(
        out.as_ref(),
        &[0, 0, 0, 1, 0x40, 0, 0, 0, 1, 0x42, 0, 0, 0, 1, 0x44],
        "SEI (39) is skipped"
    );
}

/// Real MP4 (AVCC) extradata from e2e/test.mp4 yields SPS + PPS.
#[test]
fn test_parameter_sets_from_test_mp4() -> anyhow::Result<()> {
    let path = crate::test_mp4_path();
    if !path.exists() {
        return Ok(());
    }
    let input = crate::input::AvInput::new(path.to_str().unwrap_or_default(), None, None)?;
    let video = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video"))?;
    let sets = parameter_sets_annexb(video.parameters()).expect("AVCC parameter sets");
    let nal_types: Vec<u8> = sets
        .windows(5)
        .filter(|w| w[..4] == [0, 0, 0, 1])
        .map(|w| w[4] & 0x1f)
        .collect();
    assert_eq!(nal_types, vec![7, 8], "SPS then PPS");
    Ok(())
}

#[test]
fn test_parameter_sets_from_annexb_input() -> anyhow::Result<()> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/dummy-camera/assets/sample.h264");
    let input = crate::input::AvInput::new(path.to_str().unwrap_or_default(), None, None)?;
    let video = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video"))?;
    let original = get_extradata(video.parameters()).expect("Annex B extradata");
    assert!(is_annexb_packet(original));
    assert_eq!(
        parameter_sets_annexb(video.parameters())
            .expect("sets")
            .as_ref(),
        original
    );
    Ok(())
}

#[test]
fn test_aac_adts_header_uses_audio_specific_config_and_checks_limits() {
    let header = aac_adts_header(&[0x11, 0x88], 100).expect("AAC-LC 48kHz mono");
    assert_eq!(header, [0xff, 0xf1, 0x4c, 0x40, 0x0d, 0x7f, 0xfc]);
    assert!(aac_adts_header(&[], 100).is_none());
    assert!(aac_adts_header(&[0x11], 100).is_none());
    assert!(aac_adts_header(&[0x11, 0x88], 8185).is_none());
    assert!(aac_adts_header(&[0x11, 0x88], usize::MAX).is_none());
    assert!(aac_adts_header(&[0x2b, 0x88], 100).is_none());
}
