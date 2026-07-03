//! A single input, kept hot: it reads + decodes continuously (via ffmpeg-bus's
//! `AvInputTask` + `DecoderTask`) and forwards its decoded video frames — tagged
//! with the source id — into the shared selector channel, whether or not it is
//! the currently active source. Keeping every source decoding means a switch
//! has a decoded frame ready immediately (no black gap).

use anyhow::Result;
use ffmpeg_bus::decoder::{Decoder, DecoderTask};
use ffmpeg_bus::frame::{RawFrame, RawFrameCmd};
use ffmpeg_bus::input::{AvInput, AvInputTask};
use ffmpeg_bus::stream::AvStream;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;

/// A decoded video frame tagged with the source it came from.
pub struct TaggedFrame {
    pub source_id: String,
    pub frame: RawFrame,
}

pub struct Source {
    pub id: String,
    /// The source's decoded video stream descriptor. The first source's stream
    /// is used as the program encoder/muxer template.
    pub video_stream: AvStream,
    input_task: AvInputTask,
    decoder_task: DecoderTask,
}

impl Source {
    /// Open `url`, start reading + decoding, and forward tagged video frames
    /// into `out`.
    pub async fn start(id: &str, url: &str, out: mpsc::Sender<TaggedFrame>) -> Result<Self> {
        let input = AvInput::new(url, None, None)?;
        let video_stream = input
            .streams()
            .values()
            .find(|s| s.is_video())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("source {id}: no video stream in {url}"))?;

        // Subscribe the decoder to the input BEFORE the input starts reading, so
        // no packets are missed.
        let input_task = AvInputTask::new();
        let decoder = Decoder::new(&video_stream)?;
        let decoder_task = DecoderTask::new();
        // Switcher keeps only the latest frame per source, so lossy is fine.
        decoder_task.start(decoder, input_task.subscribe(), false).await;

        let mut frames = decoder_task.subscribe();
        input_task.start(input).await;

        let id_owned = id.to_string();
        tokio::spawn(async move {
            loop {
                match frames.recv().await {
                    Ok(RawFrameCmd::Data(frame)) => {
                        let tagged = TaggedFrame {
                            source_id: id_owned.clone(),
                            frame,
                        };
                        // Never block a source: if the selector is behind, drop
                        // this frame rather than stall decoding.
                        let _ = out.try_send(tagged);
                    }
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
