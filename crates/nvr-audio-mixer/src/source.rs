//! One audio input, kept hot: opens a URL, decodes its audio stream
//! continuously, and lets any number of output buses [`subscribe`](AudioSource::subscribe)
//! to the decoded-frame broadcast. A single source is decoded once and shared
//! across every bus that uses it. The audio analogue of `nvr-compositor`'s
//! `Source` (which keeps only the latest *video* frame).

use anyhow::Result;
use ffmpeg_bus::decoder::{Decoder, DecoderTask};
use ffmpeg_bus::frame::RawFrameReceiver;
use ffmpeg_bus::input::{AvInput, AvInputTask};
use ffmpeg_bus::stream::AvStream;

pub struct AudioSource {
    pub id: String,
    pub url: String,
    /// The decoded audio stream, used as the encoder template for buses that
    /// include this source.
    pub audio_stream: AvStream,
    input_task: AvInputTask,
    decoder_task: DecoderTask,
}

impl AudioSource {
    /// Open `url`, pick its first audio stream, and start reading + decoding it.
    pub async fn start(id: &str, url: &str) -> Result<Self> {
        let input = AvInput::new(url, None, None)?;
        let audio_stream = input
            .streams()
            .values()
            .find(|s| s.is_audio())
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("audio source {id}: no audio stream in {url}"))?;

        let input_task = AvInputTask::new();
        let decoder = Decoder::new(&audio_stream)?;
        let decoder_task = DecoderTask::new();
        decoder_task
            .start(decoder, input_task.subscribe(), false)
            .await;
        input_task.start(input).await;

        Ok(Self {
            id: id.to_string(),
            url: url.to_string(),
            audio_stream,
            input_task,
            decoder_task,
        })
    }

    /// A fresh receiver on this source's decoded-audio broadcast. Each bus that
    /// consumes the source gets its own receiver.
    pub fn subscribe(&self) -> RawFrameReceiver {
        self.decoder_task.subscribe()
    }
}

impl Drop for AudioSource {
    fn drop(&mut self) {
        self.input_task.stop();
        self.decoder_task.stop();
    }
}
