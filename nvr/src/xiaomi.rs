//! Phase 6: lite-nvr integration for Xiaomi cameras.
//!
//! Runs the `xiaomi` crate's pull (cloud auth -> resolve -> miss/CS2 connect ->
//! start media) and pushes the decoded H264 stream into a ZLMediaKit `Media`, so
//! the rest of lite-nvr (live FLV/HLS, recording, playback) works with it like
//! any other device.
//!
//! Video-only for now; audio (PCMA/Opus) is a follow-up once the ZLM audio
//! track codecs are wired. UNTESTED until run against a real camera.
//!
//! The pull function is ready; wiring it to a `xiaomi` device input type +
//! the manager lifecycle + the dashboard is the remaining follow-up, so the
//! items are currently `allow(dead_code)`.
#![allow(dead_code)]

use std::sync::Arc;

use rszlm::{
    frame::Frame as ZlmFrame,
    media::Media,
    obj::{CodecArgs, CodecId, Track, VideoCodecArgs},
};
use tokio_util::sync::CancellationToken;
use xiaomi::{cloud::Cloud, device, miss};

/// Everything needed to pull one Xiaomi camera. `token` is the `passToken`
/// obtained from a prior account login (`Cloud::login` -> `user_token`).
#[derive(Clone, Debug)]
pub struct XiaomiConfig {
    pub user_id: String,
    pub token: String,
    /// Region: "" (mainland China) or de/i2/ru/sg/us.
    pub region: String,
    pub did: String,
    pub model: String,
    /// Camera address (its local IP from the device list).
    pub ip: String,
}

/// Spawn a blocking worker that streams the camera into `media` until `cancel`.
pub fn spawn_to_zlm(cfg: XiaomiConfig, media: Arc<Media>, cancel: CancellationToken) {
    std::thread::spawn(move || {
        if let Err(e) = run(&cfg, &media, &cancel) {
            log::error!("xiaomi: device {} stream ended: {:#}", cfg.did, e);
        }
    });
}

fn run(cfg: &XiaomiConfig, media: &Arc<Media>, cancel: &CancellationToken) -> anyhow::Result<()> {
    let mut cloud = Cloud::new(device::APP_XIAOMI_HOME)?;
    cloud.login_with_token(&cfg.user_id, &cfg.token)?;

    let conn = device::resolve_miss(&cloud, &cfg.region, &cfg.did, &cfg.model)?;
    log::info!(
        "xiaomi: device {} resolved (vendor={}, model={})",
        cfg.did,
        conn.vendor,
        cfg.model
    );

    let client = miss::Client::connect(&cfg.ip, &conn, "")?;
    // audio off for the video-only MVP
    client.start_media("", "", "0")?;

    let mut track_inited = false;
    while !cancel.is_cancelled() {
        let pkt = client.read_packet()?;
        if pkt.codec_id != miss::CODEC_H264 {
            continue;
        }

        if !track_inited {
            // ZLM derives the real dimensions from the in-band SPS.
            let track = Track::new(
                CodecId::H264,
                Some(CodecArgs::Video(VideoCodecArgs {
                    width: 0,
                    height: 0,
                    fps: 0.0,
                })),
            );
            media.init_track(&track);
            media.init_complete();
            track_inited = true;
            log::info!("xiaomi: device {} video track initialized", cfg.did);
        }

        // Camera emits Annex B H264; ZLM accepts it directly.
        let frame = ZlmFrame::new(CodecId::H264, pkt.timestamp, pkt.timestamp, &pkt.payload);
        if !media.input_frame(&frame) {
            log::warn!("xiaomi: device {} input_frame failed", cfg.did);
        }
    }

    let _ = client.stop_media();
    log::info!("xiaomi: device {} stream cancelled", cfg.did);
    Ok(())
}
