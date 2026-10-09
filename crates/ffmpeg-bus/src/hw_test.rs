use ffmpeg_next::codec::Id;

use super::{mark_runtime_failure, video_decoder_candidates, video_encoder_candidates};

#[cfg(feature = "rockchip")]
#[test]
fn test_rockchip_codecs_preferred_with_software_fallback() {
    for (request, name, software) in [
        ("h264", "h264_rkmpp", "libx264"),
        ("hevc", "hevc_rkmpp", "libx265"),
        ("mjpeg", "mjpeg_rkmpp", "mjpeg"),
    ] {
        for request in [request, name] {
            let candidates = video_encoder_candidates(Some(request));
            assert_eq!(candidates[0].name, name);
            assert!(candidates[0].is_hw);
            assert!(candidates.iter().any(|c| c.name == software && !c.is_hw));
        }
    }
    for (id, name) in [
        (Id::H264, "h264_rkmpp"),
        (Id::HEVC, "hevc_rkmpp"),
        (Id::AV1, "av1_rkmpp"),
    ] {
        let candidates = video_decoder_candidates(id);
        assert_eq!(candidates[0].name, name);
        assert!(candidates[0].is_hw);
    }
}

#[cfg(not(feature = "rockchip"))]
#[test]
fn test_default_candidates_do_not_include_rockchip() {
    assert!(
        video_encoder_candidates(None)
            .iter()
            .all(|c| !c.name.ends_with("_rkmpp"))
    );
    assert!(
        video_decoder_candidates(Id::H264)
            .iter()
            .all(|c| !c.name.ends_with("_rkmpp"))
    );
}

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
