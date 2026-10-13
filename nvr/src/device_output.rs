//! Map persisted per-device output settings into the shared media pipeline.
use media_pipe_core::{EncodeConfig, OutputConfig};
use nvr_db::device::{AudioCodec, DeviceInfo, OutputConfig as Settings, VideoCodec};

pub(crate) async fn outputs(
    id: &str,
    media: std::sync::Arc<rszlm::media::Media>,
    include_original: bool,
    settings: Option<&Settings>,
) -> anyhow::Result<Vec<OutputConfig>> {
    let use_global = settings.and_then(|s| s.use_global_audio).unwrap_or(true);
    let audio =
        crate::audiomixer::output::audio_for_output(id, include_original, use_global).await?;
    let mut outputs = media_pipe_zlm::zlm_outputs(media, include_original || audio.is_some());
    for output in &mut outputs {
        output.use_global_audio = use_global;
        output.external_audio = audio.clone();
    }
    Ok(outputs)
}

pub(crate) fn validate(device: &DeviceInfo) -> anyhow::Result<()> {
    let Some(cfg) = device.config.output.as_ref() else {
        return Ok(());
    };
    if matches!(device.input_type.as_str(), "gb28181" | "xiaomi")
        && (cfg.video_codec.is_some()
            || cfg.width.is_some()
            || cfg.height.is_some()
            || cfg.fps.is_some()
            || cfg.max_bitrate.is_some()
            || cfg.audio_codec.is_some()
            || cfg.sample_rate.is_some())
    {
        anyhow::bail!("该设备使用原生转发，暂不支持输出转码参数");
    }
    if cfg.width.is_some() != cfg.height.is_some() {
        anyhow::bail!("输出宽度和高度必须同时设置");
    }
    for size in [cfg.width, cfg.height].into_iter().flatten() {
        anyhow::ensure!(
            (2..=8192).contains(&size) && size % 2 == 0,
            "输出分辨率必须是 2～8192 的偶数"
        );
    }
    if let Some(fps) = cfg.fps {
        anyhow::ensure!((1..=120).contains(&fps), "FPS 必须是 1～120");
    }
    if let Some(rate) = cfg.max_bitrate {
        anyhow::ensure!(
            (32_000..=100_000_000).contains(&rate),
            "最大码率必须是 32～100000 Kbps"
        );
    }
    if let Some(rate) = cfg.sample_rate {
        let supported: &[u32] = if cfg.audio_codec == Some(AudioCodec::Opus) {
            &[8000, 12000, 16000, 24000, 48000]
        } else {
            &[
                8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000, 64000, 88200, 96000,
            ]
        };
        anyhow::ensure!(supported.contains(&rate), "不支持所选音频采样率");
    }
    Ok(())
}

pub(crate) fn apply(outputs: &mut [OutputConfig], cfg: Option<&Settings>) {
    let Some(cfg) = cfg else {
        return;
    };
    if cfg.video_codec.is_some()
        || cfg.width.is_some()
        || cfg.fps.is_some()
        || cfg.max_bitrate.is_some()
    {
        if let Some(video) = outputs.first_mut() {
            video.encode = Some(EncodeConfig {
                codec: match cfg.video_codec {
                    Some(VideoCodec::H265) => "hevc",
                    _ => "h264",
                }
                .to_owned(),
                width: cfg.width,
                height: cfg.height,
                fps: cfg.fps,
                max_bitrate: cfg.max_bitrate,
                downscale_only: true,
                pixel_format: Some("yuv420p".to_owned()),
                ..EncodeConfig::default()
            });
        }
    }
    if (cfg.audio_codec.is_some() || cfg.sample_rate.is_some())
        && outputs.iter().all(|o| o.external_audio.is_none())
    {
        if let Some(audio) = outputs.get_mut(1) {
            audio.encode = Some(EncodeConfig {
                codec: match cfg.audio_codec {
                    Some(AudioCodec::Opus) => "libopus",
                    _ => "aac",
                }
                .to_owned(),
                sample_rate: cfg.sample_rate,
                ..EncodeConfig::default()
            });
        }
    }
}

#[cfg(test)]
#[path = "device_output_test.rs"]
mod device_output_test;
