//! One audio input, kept hot: opens a URL, decodes its audio stream
//! continuously, and lets any number of output buses [`subscribe`](AudioSource::subscribe)
//! to the decoded-frame broadcast. A single source is decoded once and shared
//! across every bus that uses it. The audio analogue of `nvr-compositor`'s
//! `Source` (which keeps only the latest *video* frame).

use anyhow::Result;
use ffmpeg_bus::decoder::{Decoder, DecoderTask};
use ffmpeg_bus::frame::{RawFrameCmd, RawFrameReceiver, RawFrameSender};
use ffmpeg_bus::input::{AvInput, AvInputTask};
use ffmpeg_bus::stream::AvStream;
use std::sync::Arc;

pub struct AudioSource {
    pub id: String,
    pub url: String,
    /// The decoded audio stream, used as the encoder template for buses that
    /// include this source.
    pub audio_stream: AvStream,
    input_task: Option<AvInputTask>,
    decoder_task: Option<DecoderTask>,
    frames: Option<RawFrameSender>,
    receiver_tx: Option<tokio::sync::mpsc::UnboundedSender<RawFrameReceiver>>,
    cancel: tokio_util::sync::CancellationToken,
    relay: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    capture: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    ready: Arc<std::sync::atomic::AtomicBool>,
    error: Arc<std::sync::Mutex<Option<String>>>,
}

impl AudioSource {
    /// Open `url`, pick its first audio stream, and start reading + decoding it.
    pub async fn start(id: &str, url: &str) -> Result<Self> {
        Self::start_with_format(id, url, None).await
    }

    pub async fn start_with_format(id: &str, url: &str, format: Option<&str>) -> Result<Self> {
        Self::start_with_interrupt(id, url, format, None).await
    }

    async fn start_with_interrupt(
        id: &str,
        url: &str,
        format: Option<&str>,
        interrupt: Option<Arc<std::sync::atomic::AtomicBool>>,
    ) -> Result<Self> {
        let address = url.to_string();
        let format = format.map(str::to_string);
        let input = tokio::task::spawn_blocking(move || {
            let mut options = ffmpeg_next::Dictionary::new();
            if format.as_deref() == Some("alsa") {
                options.set("sample_rate", "48000");
                options.set("channels", "2");
            }
            if address.starts_with("rtsp://") {
                options.set("rtsp_transport", "tcp");
                options.set("timeout", "5000000");
            }
            AvInput::open(&address, format.as_deref(), Some(options), interrupt)
        })
        .await??;
        let audio_stream = input
            .streams()
            .values()
            .find(|s| s.is_audio())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("audio source {id}: no audio stream in {url}"))?;

        let input_task = AvInputTask::new();
        let decoder = Decoder::new(&audio_stream)?;
        let decoder_task = DecoderTask::new();
        decoder_task.start(decoder, input_task.subscribe()).await;
        input_task.start(input).await;

        Ok(Self {
            id: id.to_string(),
            url: url.to_string(),
            audio_stream,
            input_task: Some(input_task),
            decoder_task: Some(decoder_task),
            frames: None,
            receiver_tx: None,
            cancel: tokio_util::sync::CancellationToken::new(),
            relay: std::sync::Mutex::new(None),
            capture: std::sync::Mutex::new(None),
            ready: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            error: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// A fresh receiver on this source's decoded-audio broadcast. Each bus that
    /// consumes the source gets its own receiver.
    pub fn subscribe(&self) -> RawFrameReceiver {
        if let Some(frames) = &self.frames {
            return frames.subscribe();
        }
        self.decoder_task
            .as_ref()
            .expect("decoded source owns decoder")
            .subscribe(false)
    }

    /// A stable source endpoint whose original pipe subscription can be
    /// replaced on reconnect. No second demuxer/decoder or post-mix RTSP pull.
    pub fn shared(id: &str) -> Self {
        let (frames, _) = tokio::sync::broadcast::channel(64);
        let (receiver_tx, mut replacements) = tokio::sync::mpsc::unbounded_channel();
        let cancel = tokio_util::sync::CancellationToken::new();
        let stop = cancel.clone();
        let out = frames.clone();
        let ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let status = ready.clone();
        let relay = tokio::spawn(async move {
            let mut current: Option<RawFrameReceiver> = None;
            loop {
                tokio::select! {
                    _ = stop.cancelled() => break,
                    replacement = replacements.recv() => match replacement {
                        Some(rx) => current = Some(rx),
                        None => break,
                    },
                    item = async {
                        match current.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => futures_pending().await,
                        }
                    } => match item {
                        Ok(RawFrameCmd::Data(frame)) => { status.store(true,std::sync::atomic::Ordering::Relaxed); let _ = out.send(RawFrameCmd::Data(frame)); }
                        Ok(RawFrameCmd::EOF) | Err(tokio::sync::broadcast::error::RecvError::Closed) => { current = None; status.store(false,std::sync::atomic::Ordering::Relaxed); }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    }
                }
            }
        });
        Self {
            id: id.to_string(),
            url: format!("source://{id}"),
            audio_stream: AvStream::new(
                0,
                ffmpeg_next::codec::Parameters::new(),
                ffmpeg_next::Rational(1, 48_000),
                ffmpeg_next::Rational(0, 1),
            ),
            input_task: None,
            decoder_task: None,
            frames: Some(frames),
            receiver_tx: Some(receiver_tx),
            cancel,
            relay: std::sync::Mutex::new(Some(relay)),
            capture: std::sync::Mutex::new(None),
            ready,
            error: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub fn attach(&self, receiver: RawFrameReceiver) -> Result<()> {
        self.receiver_tx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("source is not a shared pipe source"))?
            .send(receiver)
            .map_err(|_| anyhow::anyhow!("audio source stopped"))
    }

    pub fn capture(id: &str, address: &str, format: &str) -> Self {
        let mut source = Self::shared(id);
        source.url = format!("{format}://{address}");
        let id = id.to_string();
        let address = address.to_string();
        let format = format.to_string();
        let stop = source.cancel.clone();
        let replacements = source
            .receiver_tx
            .as_ref()
            .expect("shared source has receiver channel")
            .clone();
        let error = source.error.clone();
        let task = tokio::spawn(async move {
            while !stop.is_cancelled() {
                let interrupt = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let opening = Self::start_with_interrupt(
                    &id,
                    &address,
                    Some(&format),
                    Some(interrupt.clone()),
                );
                tokio::pin!(opening);
                let result = tokio::select! {
                    result = &mut opening => result,
                    _ = stop.cancelled() => {
                        interrupt.store(true, std::sync::atomic::Ordering::Release);
                        if let Ok(input) = opening.await { input.shutdown().await; }
                        break;
                    }
                };
                match result {
                    Ok(input) => {
                        *error.lock().unwrap_or_else(|e| e.into_inner()) = None;
                        let mut end = input.subscribe();
                        if replacements.send(input.subscribe()).is_err() {
                            input.shutdown().await;
                            break;
                        }
                        loop {
                            tokio::select! {
                                _ = stop.cancelled() => break,
                                r = end.recv() => match r {
                                    Ok(RawFrameCmd::EOF) | Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                                    _ => {}
                                }
                            }
                        }
                        input.shutdown().await;
                    }
                    Err(e) => {
                        *error.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("{e:#}"));
                    }
                }
                tokio::select! { _ = stop.cancelled() => break, _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {} }
            }
        });
        *source.capture.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
        source
    }

    pub fn ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub async fn shutdown(&self) {
        self.cancel.cancel();
        let capture = self
            .capture
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(capture) = capture {
            let _ = capture.await;
        }
        if let Some(input) = &self.input_task {
            input.stop_and_join().await;
        }
        if let Some(decoder) = &self.decoder_task {
            decoder.stop_and_join().await;
        }
        let relay = self.relay.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(relay) = relay {
            let _ = relay.await;
        }
    }
}

impl Drop for AudioSource {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(input) = &self.input_task {
            input.stop();
        }
        if let Some(decoder) = &self.decoder_task {
            decoder.stop();
        }
    }
}

async fn futures_pending() -> Result<RawFrameCmd, tokio::sync::broadcast::error::RecvError> {
    std::future::pending().await
}

#[cfg(test)]
#[path = "source_test.rs"]
mod source_test;
