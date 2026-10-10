use ffmpeg_next::codec::Id;
use ffmpeg_next::format::Pixel;

use super::{
    mark_runtime_failure, rkmpp_decodes, video_decoder_candidates, video_encoder_candidates,
};

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
        let candidates = video_decoder_candidates(id, Pixel::YUV420P);
        assert_eq!(candidates[0].name, name);
        assert!(candidates[0].is_hw);
    }
}

/// Non-4:2:0 sources (4:2:2 MJPEG, 4:4:4 HEVC) skip RKMPP for software; it
/// would otherwise take their packets and silently yield no frames.
#[cfg(feature = "rockchip")]
#[test]
fn test_rockchip_decoder_skipped_for_non_420_sources() {
    for (id, format) in [(Id::MJPEG, Pixel::YUVJ422P), (Id::HEVC, Pixel::YUV444P)] {
        let candidates = video_decoder_candidates(id, format);
        assert!(candidates.iter().all(|c| !c.name.ends_with("_rkmpp")));
    }
    let candidates = video_decoder_candidates(Id::HEVC, Pixel::YUV444P);
    assert!(candidates.iter().any(|c| c.name == "hevc" && !c.is_hw));
    assert_eq!(
        video_decoder_candidates(Id::MJPEG, Pixel::YUVJ420P)[0].name,
        "mjpeg_rkmpp"
    );
}

#[test]
fn test_rkmpp_decodes_only_420_or_unknown_formats() {
    for format in [
        Pixel::YUV420P,
        Pixel::YUVJ420P,
        Pixel::NV12,
        Pixel::YUV420P10LE,
        Pixel::None,
    ] {
        assert!(rkmpp_decodes(format), "{format:?}");
    }
    for format in [
        Pixel::YUV422P,
        Pixel::YUVJ422P,
        Pixel::YUV444P,
        Pixel::NV24,
        Pixel::GRAY8,
        Pixel::RGB24,
    ] {
        assert!(!rkmpp_decodes(format), "{format:?}");
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
        video_decoder_candidates(Id::H264, Pixel::YUV420P)
            .iter()
            .all(|c| !c.name.ends_with("_rkmpp"))
    );
}

/// A hardware codec that failed at runtime is no longer offered; software
/// candidates and other hardware ones are unaffected.
#[test]
fn test_runtime_failed_hw_codec_is_skipped() {
    assert!(
        video_decoder_candidates(Id::HEVC, Pixel::YUV420P)
            .iter()
            .any(|c| c.name == "hevc_vaapi")
    );
    mark_runtime_failure("hevc_vaapi");
    let dec = video_decoder_candidates(Id::HEVC, Pixel::YUV420P);
    assert!(!dec.iter().any(|c| c.name == "hevc_vaapi"));
    assert!(dec.iter().any(|c| c.name == "hevc_qsv"), "other hw kept");
    assert!(dec.iter().any(|c| !c.is_hw), "software kept");
    let enc = video_encoder_candidates(Some("hevc"));
    assert!(!enc.iter().any(|c| c.name == "hevc_vaapi"));
}
