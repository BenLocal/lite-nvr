use ffmpeg_next::codec::Id as CodecId;

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
        CodecCandidate::hw("h264_videotoolbox"),
        CodecCandidate::hw("h264_nvenc"),
        CodecCandidate::hw("h264_qsv"),
        CodecCandidate::hw("h264_vaapi"),
    ]
}

fn hevc_hw_candidates() -> Vec<CodecCandidate> {
    vec![
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

pub fn video_decoder_candidates(codec_id: CodecId) -> Vec<CodecCandidate> {
    let mut out = Vec::new();
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

#[cfg(test)]
#[path = "hw_test.rs"]
mod hw_test;
