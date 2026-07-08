//! One output bus: mixes the audio of the inputs assigned to it and publishes
//! the result as a single continuous stream.
//!
//! This is thin orchestration over `ffmpeg-bus`'s reusable building blocks:
//! - mixing (resample + per-input gain/mute + PCM sum) is
//!   [`DynamicMixerTask`](ffmpeg_bus::audio_mixer::DynamicMixerTask);
//! - AAC encode is [`EncoderTask`](ffmpeg_bus::encoder::EncoderTask);
//! - muxing + publish is [`AvOutput`](ffmpeg_bus::output::AvOutput).
//!
//! Chain:  sources → mixer.add_input → mixer.subscribe() → EncoderTask →
//!         EncoderTask.subscribe() → publish task → AvOutput(flv) → ZLM.

use ffmpeg_bus::audio_mixer::DynamicMixerTask;
use ffmpeg_bus::encoder::{AudioSettings, Encoder, EncoderTask};
use ffmpeg_bus::frame::RawFrameReceiver;
use ffmpeg_bus::output::AvOutput;
use ffmpeg_bus::packet::{RawPacketCmd, RawPacketReceiver};
use ffmpeg_bus::stream::AvStream;
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

use crate::InputSnapshot;

/// Mixer + output working sample rate.
const SAMPLE_RATE: u32 = 48_000;
/// Output channel count (stereo).
const CHANNELS: u32 = 2;
/// Output AAC bitrate (bps).
const OUTPUT_BITRATE: u64 = 128_000;

/// Default per-input volume when an input is added (unity gain).
pub const DEFAULT_VOLUME: u32 = ffmpeg_bus::audio_mixer::DEFAULT_VOLUME;

/// A running output bus. Dropping it stops the mixer, encoder and publisher.
pub struct MixBus {
    id: String,
    publish_url: String,
    mixer: DynamicMixerTask,
    enc_task: EncoderTask,
    publish_cancel: CancellationToken,
}

impl MixBus {
    /// Start the bus: mixer → AAC encoder → publish to `publish_url` (FLV/RTMP).
    /// `template` supplies the encoder's codec context (any input source's audio
    /// stream will do). Publishes silence until inputs are added.
    pub async fn start(id: &str, publish_url: &str, template: AvStream) -> anyhow::Result<Self> {
        let mixer = DynamicMixerTask::new(SAMPLE_RATE);
        mixer.start();
        let mixed_rx = mixer.subscribe();

        let settings = AudioSettings {
            sample_rate: Some(SAMPLE_RATE),
            channels: Some(CHANNELS),
            bitrate: Some(OUTPUT_BITRATE),
            ..Default::default()
        };
        let encoder = Encoder::new_audio(&template, settings, None)?;
        // Grab the muxer stream description before the encoder is moved into the task.
        let out_stream = encoder.output_stream(0);
        let enc_task = EncoderTask::new();
        // Live output: lossy (drop under back-pressure) rather than stall the mix.
        enc_task.start(encoder, mixed_rx, false).await;
        let packet_rx = enc_task.subscribe();

        let publish_cancel = CancellationToken::new();
        spawn_publish(
            id.to_string(),
            publish_url.to_string(),
            out_stream,
            packet_rx,
            publish_cancel.clone(),
        );

        Ok(Self {
            id: id.to_string(),
            publish_url: publish_url.to_string(),
            mixer,
            enc_task,
            publish_cancel,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn publish_url(&self) -> &str {
        &self.publish_url
    }

    /// Add (or replace) an input on this bus at the given volume.
    pub fn add_input(&self, source_id: &str, receiver: RawFrameReceiver, volume: u32) {
        self.mixer.add_input(source_id, receiver, volume);
    }

    pub fn remove_input(&self, source_id: &str) -> anyhow::Result<()> {
        self.mixer.remove_input(source_id)
    }

    pub fn set_volume(&self, source_id: &str, volume: u32) -> anyhow::Result<()> {
        self.mixer.set_volume(source_id, volume)
    }

    pub fn set_muted(&self, source_id: &str, muted: bool) -> anyhow::Result<()> {
        self.mixer.set_muted(source_id, muted)
    }

    /// Snapshot of this bus's inputs for the API / persistence.
    pub fn inputs_snapshot(&self) -> Vec<InputSnapshot> {
        self.mixer
            .inputs()
            .into_iter()
            .map(|(source_id, volume, muted)| InputSnapshot {
                source_id,
                volume,
                muted,
            })
            .collect()
    }
}

impl Drop for MixBus {
    fn drop(&mut self) {
        self.mixer.cancel();
        self.enc_task.stop();
        self.publish_cancel.cancel();
    }
}

/// Consume the encoder's packets on a blocking thread and mux them to the
/// publish URL. Ends when the packet stream closes (mixer/encoder stopped) or
/// `cancel` fires.
fn spawn_publish(
    id: String,
    publish_url: String,
    out_stream: AvStream,
    mut packet_rx: RawPacketReceiver,
    cancel: CancellationToken,
) {
    tokio::task::spawn_blocking(move || {
        let mut output = match AvOutput::new(&publish_url, Some("flv"), None) {
            Ok(output) => output,
            Err(e) => {
                log::error!("audio bus '{id}' open output {publish_url}: {e:#}");
                return;
            }
        };
        if let Err(e) = output.add_stream(&out_stream) {
            log::error!("audio bus '{id}' add_stream: {e:#}");
            return;
        }
        log::info!("audio bus '{id}' publishing -> {publish_url}");

        while !cancel.is_cancelled() {
            match packet_rx.blocking_recv() {
                Ok(RawPacketCmd::Data(pkt)) => {
                    if let Err(e) = output.write_packet(0, pkt) {
                        log::warn!("audio bus '{id}' write_packet: {e:#}");
                    }
                }
                Ok(RawPacketCmd::EOF) => break,
                Err(RecvError::Lagged(n)) => {
                    log::warn!("audio bus '{id}' publish lagged, dropped {n}");
                }
                Err(RecvError::Closed) => break,
            }
        }
        let _ = output.finish();
        log::info!("audio bus '{id}' publish stopped");
    });
}
