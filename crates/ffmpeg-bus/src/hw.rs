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

fn dedup_by_name(candidates: Vec<CodecCandidate>) -> Vec<CodecCandidate> {
    let mut out = Vec::with_capacity(candidates.len());
    for c in candidates {
        if out.iter().any(|x: &CodecCandidate| x.name == c.name) {
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
    dedup_by_name(out)
}
