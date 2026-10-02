use ffmpeg_next::codec::Id;

use super::{mark_runtime_failure, video_decoder_candidates, video_encoder_candidates};

/// A hardware codec that failed at runtime is no longer offered; software
/// candidates and other hardware ones are unaffected.
#[test]
fn test_runtime_failed_hw_codec_is_skipped() {
    assert!(
        video_decoder_candidates(Id::HEVC)
            .iter()
            .any(|c| c.name == "hevc_vaapi")
    );
    mark_runtime_failure("hevc_vaapi");
    let dec = video_decoder_candidates(Id::HEVC);
    assert!(!dec.iter().any(|c| c.name == "hevc_vaapi"));
    assert!(dec.iter().any(|c| c.name == "hevc_qsv"), "other hw kept");
    assert!(dec.iter().any(|c| !c.is_hw), "software kept");
    let enc = video_encoder_candidates(Some("hevc"));
    assert!(!enc.iter().any(|c| c.name == "hevc_vaapi"));
}
