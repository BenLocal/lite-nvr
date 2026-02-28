//! Hardware-accelerated codec discovery.
//!
//! Provides functions to find hardware decoders and encoders (CUDA/VAAPI/QSV/V4L2M2M)
//! with automatic fallback to software codecs when not available.

/// Try to find a hardware-accelerated decoder for the given codec ID.
/// Returns the first available hardware decoder, or None if none is found.
pub fn find_hw_decoder(codec_id: ffmpeg_next::codec::Id) -> Option<ffmpeg_next::Codec> {
    let hw_names: &[&str] = match codec_id {
        ffmpeg_next::codec::Id::H264 => &["h264_cuvid", "h264_qsv", "h264_v4l2m2m"],
        ffmpeg_next::codec::Id::HEVC => &["hevc_cuvid", "hevc_qsv", "hevc_v4l2m2m"],
        ffmpeg_next::codec::Id::VP8 => &["vp8_cuvid", "vp8_qsv", "vp8_v4l2m2m"],
        ffmpeg_next::codec::Id::VP9 => &["vp9_cuvid", "vp9_qsv", "vp9_v4l2m2m"],
        ffmpeg_next::codec::Id::AV1 => &["av1_cuvid", "av1_qsv"],
        ffmpeg_next::codec::Id::MPEG2VIDEO => &["mpeg2_cuvid", "mpeg2_qsv", "mpeg2_v4l2m2m"],
        ffmpeg_next::codec::Id::MPEG4 => &["mpeg4_cuvid", "mpeg4_v4l2m2m"],
        _ => &[],
    };

    for name in hw_names {
        if let Some(codec) = ffmpeg_next::decoder::find_by_name(name) {
            log::info!("found hardware decoder: {}", name);
            return Some(codec);
        }
    }
    None
}

/// Try to find a hardware-accelerated encoder for the given software codec name.
/// Returns the first available hardware encoder, or None if none is found.
pub fn find_hw_encoder(codec_name: &str) -> Option<ffmpeg_next::Codec> {
    let hw_names: &[&str] = match codec_name {
        "libx264" | "h264" => &[
            "h264_nvenc",
            "h264_vaapi",
            "h264_qsv",
            "h264_v4l2m2m",
        ],
        "libx265" | "hevc" | "h265" => &[
            "hevc_nvenc",
            "hevc_vaapi",
            "hevc_qsv",
            "hevc_v4l2m2m",
        ],
        "libvpx" | "libvpx-vp9" | "vp9" => &["vp9_vaapi", "vp9_qsv"],
        "libaom-av1" | "libsvtav1" | "av1" => &["av1_nvenc", "av1_vaapi", "av1_qsv"],
        _ => &[],
    };

    for name in hw_names {
        if let Some(codec) = ffmpeg_next::encoder::find_by_name(name) {
            log::info!("found hardware encoder: {}", name);
            return Some(codec);
        }
    }
    None
}

/// Returns a pixel format suitable for the encoder. Source formats not supported (e.g. rgb24)
/// are mapped to YUV420P; hardware encoders may prefer NV12.
pub fn pixel_format_for_encoder(
    source: ffmpeg_next::format::Pixel,
    codec_name: &str,
) -> ffmpeg_next::format::Pixel {
    use ffmpeg_next::format::Pixel;
    // Hardware encoders (nvenc, vaapi, qsv) commonly prefer NV12
    let is_hw = codec_name.contains("nvenc")
        || codec_name.contains("vaapi")
        || codec_name.contains("qsv")
        || codec_name.contains("v4l2m2m");
    match source {
        Pixel::RGB24 | Pixel::BGR24 => {
            if is_hw {
                Pixel::NV12
            } else {
                Pixel::YUV420P
            }
        }
        _ => {
            if is_hw && source == Pixel::YUV420P {
                // Most hw encoders accept YUV420P too, keep it
                source
            } else {
                source
            }
        }
    }
}

/// Backward-compatible alias for `pixel_format_for_encoder` with libx264.
pub fn pixel_format_for_libx264(source: ffmpeg_next::format::Pixel) -> ffmpeg_next::format::Pixel {
    pixel_format_for_encoder(source, "libx264")
}
