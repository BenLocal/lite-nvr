//! One input, kept hot: reads + decodes continuously and publishes its *latest*
//! decoded video frame into a shared cell. The compositor samples that cell on
//! its own output clock (hold-last-frame), so sources may run at any frame rate.
//!
//! The background frame pump is *reconnecting*: after a stream ends or drops it
//! reopens the URL, so a source that was online at create time self-heals if it
//! later goes away, exactly like one that was offline at create time (see
//! [`spawn_reconnecting`]). Both paths share the one [`reconnect_loop`].

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use ffmpeg_bus::decoder::{Decoder, DecoderTask};
use ffmpeg_bus::frame::{RawFrame, RawFrameCmd, RawFrameReceiver};
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
    /// Cancels the background reconnecting pump; fired on [`Drop`].
    cancel: CancellationToken,
}

impl Source {
    /// Open `url` once (fails if that first open fails) and keep the returned
    /// [`Source`] hot: the initial connection's video stream is exposed as
    /// [`video_stream`](Self::video_stream) for the encoder template, while a
    /// background [`reconnect_loop`] pumps decoded frames into `latest` and
    /// reconnects on EOF/error until the `Source` is dropped.
    pub async fn start(id: &str, url: &str) -> Result<Self> {
        // First open must succeed: it yields the encoder's stream template.
        let conn = open_connection(id, url).await?;
        let video_stream = conn.video_stream.clone();

        let latest: LatestFrame = Arc::new(Mutex::new(None));
        let cancel = CancellationToken::new();
        // Pump the connection we just opened, then reconnect on any later drop.
        tokio::spawn(reconnect_loop(
            id.to_string(),
            url.to_string(),
            latest.clone(),
            cancel.clone(),
            Some(conn),
        ));

        Ok(Self {
            id: id.to_string(),
            video_stream,
            latest,
            cancel,
        })
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        // Stops the reconnect loop, which stops the live connection's tasks.
        self.cancel.cancel();
    }
}

/// Base delay before a compositor source's first reconnection attempts; the
/// backoff starts here right after a drop and doubles per consecutive failure.
const RECONNECT_BASE: Duration = Duration::from_secs(3);
/// Ceiling for the reconnect backoff, so a long-dead source is retried at most
/// this often (and stays quiet in the logs).
const RECONNECT_MAX: Duration = Duration::from_secs(30);
/// Cap on the backoff exponent so the `1 << exp` shift can never overflow;
/// `RECONNECT_MAX` clamps the result well before this is reached anyway.
const RECONNECT_BACKOFF_EXP_CAP: u32 = 16;

/// Delay before the next reconnect attempt given the count of *consecutive*
/// failed opens so far. `fails == 0` (a fresh drop, or right after a success)
/// waits [`RECONNECT_BASE`] for a fast reconnect; each further consecutive
/// failure doubles the delay, capped at [`RECONNECT_MAX`].
fn backoff_delay(fails: u32) -> Duration {
    let exp = fails.saturating_sub(1).min(RECONNECT_BACKOFF_EXP_CAP);
    let factor = 1u32 << exp; // 2^exp; exp <= cap, so this never overflows.
    RECONNECT_BASE.saturating_mul(factor).min(RECONNECT_MAX)
}

/// Keep a frame cell fed from `url`, reconnecting until `cancel` fires. Returns
/// the shared [`LatestFrame`] immediately (initially empty → a black tile) and
/// drives it in the background.
///
/// Unlike [`Source::start`], this never fails on an unreachable URL: it is for
/// compositor sources that were offline when the program was created. The cell
/// is placed in the pool up-front so the source is switchable right away, and
/// its picture appears the moment it comes up (and recovers if it later drops).
/// Open/decode errors are logged and retried. Both this and [`Source::start`]
/// drive the same [`reconnect_loop`]; they differ only in whether the first
/// open is required to succeed.
pub fn spawn_reconnecting(id: &str, url: &str, cancel: CancellationToken) -> LatestFrame {
    let latest: LatestFrame = Arc::new(Mutex::new(None));
    tokio::spawn(reconnect_loop(
        id.to_string(),
        url.to_string(),
        latest.clone(),
        cancel,
        None,
    ));
    latest
}

/// A single open input + its running decode tasks and frame stream.
struct Connection {
    video_stream: AvStream,
    input_task: AvInputTask,
    decoder_task: DecoderTask,
    frames: RawFrameReceiver,
}

/// Reconnecting pump shared by [`Source::start`] and [`spawn_reconnecting`]:
/// pump each connection's frames into `latest`, and when it ends (EOF/error)
/// reopen `url` after a [`backoff_delay`], until `cancel` fires. `first` is an
/// already-open connection to pump before the first reopen (used by
/// [`Source::start`], whose first open must succeed up-front); pass `None` to
/// open lazily inside the loop (used by [`spawn_reconnecting`]).
///
/// The backoff keeps a long-dead source (and the external `404`/`no such
/// stream` noise its opens provoke) quiet: the first failure logs at `warn!`
/// (so a drop is visible), later consecutive failures at `debug!`, and the
/// retry interval grows from [`RECONNECT_BASE`] up to [`RECONNECT_MAX`].
async fn reconnect_loop(
    id: String,
    url: String,
    latest: LatestFrame,
    cancel: CancellationToken,
    mut first: Option<Connection>,
) {
    let mut fails: u32 = 0;
    while !cancel.is_cancelled() {
        match first.take() {
            Some(conn) => {
                // The already-open connection counts as a success.
                fails = 0;
                pump_connection(&id, conn, &latest, &cancel).await;
            }
            None => match open_connection(&id, &url).await {
                Ok(conn) => {
                    fails = 0;
                    pump_connection(&id, conn, &latest, &cancel).await;
                }
                Err(e) => {
                    fails = fails.saturating_add(1);
                    if fails == 1 {
                        log::warn!("compositor source {id}: connect failed ({e:#})");
                    } else {
                        log::debug!(
                            "compositor source {id}: connect failed ({e:#}), attempt {fails}"
                        );
                    }
                }
            },
        }
        // Back off before the next attempt: fast right after a drop
        // (`fails == 0`), then exponentially up to `RECONNECT_MAX` while it
        // keeps failing. Cancellation stays instant.
        let delay = backoff_delay(fails);
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(delay) => {}
        }
    }
    log::info!("compositor source {id}: reconnect loop stopped");
}

/// Open `url`, start its input + decoder tasks, and return the live connection.
/// `Err` if the input can't open or has no video stream.
async fn open_connection(id: &str, url: &str) -> Result<Connection> {
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
    let frames = decoder_task.subscribe();
    input_task.start(input).await;
    log::info!("compositor source {id}: connected");

    Ok(Connection {
        video_stream,
        input_task,
        decoder_task,
        frames,
    })
}

/// Pump one connection's decoded video into `latest` until EOF, a fatal error,
/// or `cancel`, then stop its input/decoder tasks.
async fn pump_connection(
    id: &str,
    conn: Connection,
    latest: &LatestFrame,
    cancel: &CancellationToken,
) {
    let Connection {
        input_task,
        decoder_task,
        mut frames,
        ..
    } = conn;
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
}

#[cfg(test)]
#[path = "source_test.rs"]
mod source_test;
