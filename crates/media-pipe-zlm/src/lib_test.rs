use ffmpeg_next::codec::Id;
use rszlm::obj::CodecId;

use super::{video_payload, zlm_codec_id};

#[test]
fn test_zlm_codec_id_maps_camera_codecs() {
    assert!(matches!(zlm_codec_id(Id::H264), Some(CodecId::H264)));
    assert!(matches!(zlm_codec_id(Id::HEVC), Some(CodecId::H265)));
    assert!(matches!(zlm_codec_id(Id::AAC), Some(CodecId::AAC)));
    assert!(matches!(zlm_codec_id(Id::PCM_ALAW), Some(CodecId::G711A)));
    assert!(matches!(zlm_codec_id(Id::PCM_MULAW), Some(CodecId::G711U)));
    assert!(matches!(zlm_codec_id(Id::OPUS), Some(CodecId::Opus)));
}

#[test]
fn test_zlm_codec_id_rejects_unsupported() {
    assert!(zlm_codec_id(Id::ADPCM_G726).is_none());
    assert!(zlm_codec_id(Id::MP3).is_none());
}

#[test]
fn test_video_payload_annexb_passthrough() {
    let annexb = [0, 0, 0, 1, 0x65, 0x88];
    let out = video_payload(&annexb, true, false, Some(&[0, 0, 0, 1, 0x67]));
    assert_eq!(out.as_ref(), &annexb, "Annex B input is not touched");
}

#[test]
fn test_video_payload_avcc_keyframe_gets_parameter_sets() {
    let avcc = [0, 0, 0, 2, 0x65, 0x88];
    let sets = [0, 0, 0, 1, 0x67, 0xaa, 0, 0, 0, 1, 0x68];
    let key = video_payload(&avcc, true, true, Some(&sets));
    assert_eq!(
        key.as_ref(),
        &[
            0, 0, 0, 1, 0x67, 0xaa, 0, 0, 0, 1, 0x68, 0, 0, 0, 1, 0x65, 0x88
        ]
    );
    let delta = video_payload(&[0, 0, 0, 2, 0x41, 0x9a], false, true, Some(&sets));
    assert_eq!(
        delta.as_ref(),
        &[0, 0, 0, 1, 0x41, 0x9a],
        "non-key: no sets"
    );
}
