//! One input, kept hot: reads + decodes continuously and publishes its *latest*
//! decoded video frame into a shared cell. The compositor samples that cell on
//! its own output clock (hold-last-frame), so sources may run at any frame rate.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use ffmpeg_bus::decoder::{Decoder, DecoderTask};
use ffmpeg_bus::frame::{RawFrame, RawFrameCmd};
use ffmpeg_bus::input::{AvInput, AvInputTask};
use ffmpeg_bus::stream::AvStream;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

/// The latest decoded video frame from a source (None until the first frame).
pub type LatestFrame = Arc<Mutex<Option<RawFrame>>>;

pub struct Source {
    pub id: String,
    pub video_stream: AvStream,
    pub latest: LatestFrame,
    input_task: AvInputTask,
    decoder_task: DecoderTask,
}

impl Source {
    pub async fn start(id: &str, url: &str) -> Result<Self> {
        let input = AvInput::new(url, None, None)?;
        let video_stream = input
            .streams()
            .values()
            .find(|s| s.is_video())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("source {id}: no video stream in {url}"))?;

        let input_task = AvInputTask::new();
        let decoder = Decoder::new(&video_stream)?;
        let decoder_task = DecoderTask::new();
        // Compositor keeps only the latest frame per source, so lossy is fine.
        decoder_task.start(decoder, input_task.subscribe(), false).await;

        let mut frames = decoder_task.subscribe();
        input_task.start(input).await;

        let latest: LatestFrame = Arc::new(Mutex::new(None));
        let sink = latest.clone();
        let id_owned = id.to_string();
        tokio::spawn(async move {
            loop {
                match frames.recv().await {
                    Ok(RawFrameCmd::Data(frame @ RawFrame::Video(_))) => {
                        *sink.lock().unwrap() = Some(frame);
                    }
                    Ok(RawFrameCmd::Data(_)) => {} // ignore audio
                    Ok(RawFrameCmd::EOF) => {
                        log::info!("source {id_owned}: end of stream");
                        break;
                    }
                    Err(RecvError::Lagged(n)) => {
                        log::warn!("source {id_owned}: decoder lagged, dropped {n} frames");
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        });

        Ok(Self {
            id: id.to_string(),
            video_stream,
            latest,
            input_task,
            decoder_task,
        })
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        self.input_task.stop();
        self.decoder_task.stop();
    }
}

/// Delay between a compositor source's reconnection attempts.
const RECONNECT_RETRY: Duration = Duration::from_secs(3);

/// Keep a frame cell fed from `url`, reconnecting until `cancel` fires. Returns
/// the shared [`LatestFrame`] immediately (initially empty → a black tile) and
/// drives it in the background.
///
/// Unlike [`Source::start`], this never fails on an unreachable URL: it is for
/// compositor sources that were offline when the program was created. The cell
/// is placed in the pool up-front so the source is switchable right away, and
/// its picture appears the moment it comes up (and recovers if it later drops).
/// Open/decode errors are logged and retried.
pub fn spawn_reconnecting(id: &str, url: &str, cancel: CancellationToken) -> LatestFrame {
    let latest: LatestFrame = Arc::new(Mutex::new(None));
    let cell = latest.clone();
    let id = id.to_string();
    let url = url.to_string();
    tokio::spawn(async move {
        while !cancel.is_cancelled() {
            if let Err(e) = pump_once(&id, &url, &cell, &cancel).await {
                log::warn!(
                    "compositor source {id}: connect failed ({e:#}); retry in {}s",
                    RECONNECT_RETRY.as_secs()
                );
            }
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(RECONNECT_RETRY) => {}
            }
        }
        log::info!("compositor source {id}: reconnect loop stopped");
    });
    latest
}

/// One connection: open `url` and decode video into `latest` until EOF, a fatal
/// error, or `cancel`. `Ok` on a clean end; `Err` if the input could not open.
async fn pump_once(
    id: &str,
    url: &str,
    latest: &LatestFrame,
    cancel: &CancellationToken,
) -> Result<()> {
    let input = AvInput::new(url, None, None)?;
    let video_stream = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("source {id}: no video stream in {url}"))?;

    let input_task = AvInputTask::new();
    let decoder = Decoder::new(&video_stream)?;
    let decoder_task = DecoderTask::new();
    decoder_task.start(decoder, input_task.subscribe(), false).await;
    let mut frames = decoder_task.subscribe();
    input_task.start(input).await;
    log::info!("compositor source {id}: connected");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            r = frames.recv() => match r {
                Ok(RawFrameCmd::Data(frame @ RawFrame::Video(_))) => {
                    *latest.lock().unwrap() = Some(frame);
                }
                Ok(RawFrameCmd::Data(_)) => {} // ignore audio
                Ok(RawFrameCmd::EOF) => {
                    log::info!("compositor source {id}: end of stream");
                    break;
                }
                Err(RecvError::Lagged(n)) => {
                    log::warn!("compositor source {id}: decoder lagged, dropped {n} frames");
                }
                Err(RecvError::Closed) => break,
            }
        }
    }
    input_task.stop();
    decoder_task.stop();
    Ok(())
}
