use ffmpeg_next::codec::Id as CodecId;
use ffmpeg_next::format::Pixel;

#[derive(Clone, Debug)]
pub struct CodecCandidate {
    pub name: String,
    pub is_hw: bool,
}

impl CodecCandidate {
    pub fn hw(name: &str) -> Self {
        Self {
            name: name.to_string(),
            is_hw: true,
        }
    }

    pub fn sw(name: &str) -> Self {
        Self {
            name: name.to_string(),
            is_hw: false,
        }
    }
}

fn h264_hw_candidates() -> Vec<CodecCandidate> {
    vec![
        #[cfg(feature = "rockchip")]
        CodecCandidate::hw("h264_rkmpp"),
        CodecCandidate::hw("h264_videotoolbox"),
        CodecCandidate::hw("h264_nvenc"),
        CodecCandidate::hw("h264_qsv"),
        CodecCandidate::hw("h264_vaapi"),
    ]
}

fn hevc_hw_candidates() -> Vec<CodecCandidate> {
    vec![
        #[cfg(feature = "rockchip")]
        CodecCandidate::hw("hevc_rkmpp"),
        CodecCandidate::hw("hevc_videotoolbox"),
        CodecCandidate::hw("hevc_nvenc"),
        CodecCandidate::hw("hevc_qsv"),
        CodecCandidate::hw("hevc_vaapi"),
    ]
}

fn h264_sw_candidates() -> Vec<CodecCandidate> {
    vec![CodecCandidate::sw("libx264"), CodecCandidate::sw("h264")]
}

fn hevc_sw_candidates() -> Vec<CodecCandidate> {
    vec![CodecCandidate::sw("libx265"), CodecCandidate::sw("hevc")]
}

/// Hardware codecs that opened fine but then failed on real data in this
/// process (e.g. QSV "MFX session" errors). Skipped from then on, so every new
/// stream does not pay the open-fail-downgrade cycle again.
static RUNTIME_FAILED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

fn runtime_failed() -> std::sync::MutexGuard<'static, std::collections::HashSet<String>> {
    // Only inserts/lookups happen under this lock: a poisoned set is still valid.
    RUNTIME_FAILED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Record that hardware codec `name` failed at runtime; later candidate lists
/// leave it out for the rest of the process.
pub fn mark_runtime_failure(name: &str) {
    if runtime_failed().insert(name.to_string()) {
        log::warn!("{name} failed at runtime; skipping it for the rest of this process");
    }
}

/// Drop duplicates, and hardware codecs known to fail at runtime.
fn dedup_by_name(candidates: Vec<CodecCandidate>) -> Vec<CodecCandidate> {
    let failed = runtime_failed();
    let mut out = Vec::with_capacity(candidates.len());
    for c in candidates {
        if out.iter().any(|x: &CodecCandidate| x.name == c.name)
            || (c.is_hw && failed.contains(&c.name))
        {
            continue;
        }
        out.push(c);
    }
    out
}

pub fn video_encoder_candidates(requested: Option<&str>) -> Vec<CodecCandidate> {
    let req = requested.unwrap_or("h264");
    let mut out = Vec::new();
    match req {
        "h264" | "avc" | "libx264" => {
            out.extend(h264_hw_candidates());
            out.extend(h264_sw_candidates());
        }
        "hevc" | "h265" | "libx265" => {
            out.extend(hevc_hw_candidates());
            out.extend(hevc_sw_candidates());
        }
        #[cfg(feature = "rockchip")]
        "h264_rkmpp" => {
            out.push(CodecCandidate::hw(req));
            out.extend(h264_sw_candidates());
        }
        #[cfg(feature = "rockchip")]
        "hevc_rkmpp" => {
            out.push(CodecCandidate::hw(req));
            out.extend(hevc_sw_candidates());
        }
        #[cfg(feature = "rockchip")]
        "mjpeg" | "mjpeg_rkmpp" => {
            out.push(CodecCandidate::hw("mjpeg_rkmpp"));
            out.push(CodecCandidate::sw("mjpeg"));
        }
        "h264_videotoolbox" => {
            out.push(CodecCandidate::hw("h264_videotoolbox"));
            out.extend(h264_sw_candidates());
        }
        "h264_nvenc" => {
            out.push(CodecCandidate::hw("h264_nvenc"));
            out.extend(h264_sw_candidates());
        }
        "h264_qsv" => {
            out.push(CodecCandidate::hw("h264_qsv"));
            out.extend(h264_sw_candidates());
        }
        "h264_vaapi" => {
            out.push(CodecCandidate::hw("h264_vaapi"));
            out.extend(h264_sw_candidates());
        }
        "hevc_videotoolbox" => {
            out.push(CodecCandidate::hw("hevc_videotoolbox"));
            out.extend(hevc_sw_candidates());
        }
        "hevc_nvenc" => {
            out.push(CodecCandidate::hw("hevc_nvenc"));
            out.extend(hevc_sw_candidates());
        }
        "hevc_qsv" => {
            out.push(CodecCandidate::hw("hevc_qsv"));
            out.extend(hevc_sw_candidates());
        }
        "hevc_vaapi" => {
            out.push(CodecCandidate::hw("hevc_vaapi"));
            out.extend(hevc_sw_candidates());
        }
        other => out.push(CodecCandidate::sw(other)),
    }
    dedup_by_name(out)
}

#[cfg_attr(not(feature = "rockchip"), allow(unused_variables))]
pub fn video_decoder_candidates(codec_id: CodecId, pixel_format: Pixel) -> Vec<CodecCandidate> {
    let mut out = Vec::new();
    #[cfg(feature = "rockchip")]
    if let Some(name) = rockchip_decoder_name(codec_id)
        && rkmpp_decodes(pixel_format)
    {
        out.push(CodecCandidate::hw(name));
    }
    match codec_id {
        CodecId::H264 => {
            out.extend(vec![
                CodecCandidate::hw("h264_videotoolbox"),
                CodecCandidate::hw("h264_cuvid"),
                CodecCandidate::hw("h264_qsv"),
                CodecCandidate::hw("h264_vaapi"),
                CodecCandidate::sw("h264"),
            ]);
        }
        CodecId::HEVC => {
            out.extend(vec![
                CodecCandidate::hw("hevc_videotoolbox"),
                CodecCandidate::hw("hevc_cuvid"),
                CodecCandidate::hw("hevc_qsv"),
                CodecCandidate::hw("hevc_vaapi"),
                CodecCandidate::sw("hevc"),
            ]);
        }
        _ => {}
    }
    // Escape hatch: some hardware decoders (e.g. QSV) open successfully but then
    // fail at runtime on the first packet (MFX session errors), leaving no
    // software fallback. Setting FFMPEG_BUS_DISABLE_HWDEC forces software decode.
    if std::env::var_os("FFMPEG_BUS_DISABLE_HWDEC").is_some() {
        out.retain(|c| !c.is_hw);
    }
    dedup_by_name(out)
}

/// Whether RKMPP may decode a source in `pixel_format`. On non-4:2:0 sources
/// (e.g. 4:2:2 MJPEG, 4:4:4 HEVC) it can open and take packets yet never
/// yield a frame or an error, so no software fallback would trigger. An
/// unknown source format is still tried.
fn rkmpp_decodes(pixel_format: Pixel) -> bool {
    match pixel_format.descriptor() {
        Some(d) => d.nb_components() >= 3 && d.log2_chroma_w() == 1 && d.log2_chroma_h() == 1,
        None => true,
    }
}

#[cfg(feature = "rockchip")]
fn rockchip_decoder_name(codec_id: CodecId) -> Option<&'static str> {
    Some(match codec_id {
        CodecId::H264 => "h264_rkmpp",
        CodecId::HEVC => "hevc_rkmpp",
        CodecId::MJPEG => "mjpeg_rkmpp",
        CodecId::MPEG1VIDEO => "mpeg1_rkmpp",
        CodecId::MPEG2VIDEO => "mpeg2_rkmpp",
        CodecId::MPEG4 => "mpeg4_rkmpp",
        CodecId::VP8 => "vp8_rkmpp",
        CodecId::VP9 => "vp9_rkmpp",
        CodecId::AV1 => "av1_rkmpp",
        _ => return None,
    })
}

#[cfg(test)]
#[path = "hw_test.rs"]
mod hw_test;
