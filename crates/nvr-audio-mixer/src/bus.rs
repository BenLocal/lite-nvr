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

use crate::AudioProfile;
use crate::InputSnapshot;
use ffmpeg_bus::shared_audio::SharedEncodedAudio;
use std::sync::Mutex;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Mixer + output working sample rate.
const SAMPLE_RATE: u32 = 48_000;

/// Default per-input volume when an input is added (unity gain).
pub const DEFAULT_VOLUME: u32 = ffmpeg_bus::audio_mixer::DEFAULT_VOLUME;

/// A running output bus. Dropping it stops the mixer, encoder and publisher.
pub struct MixBus {
    id: String,
    publish_url: String,
    mixer: DynamicMixerTask,
    enc_task: EncoderTask,
    publish_cancel: CancellationToken,
    publish_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    encoded: SharedEncodedAudio,
    profile: AudioProfile,
    publish_interrupt: Arc<AtomicBool>,
}

impl MixBus {
    /// Start the bus: mixer → AAC encoder → publish to `publish_url` (FLV/RTMP).
    /// `template` supplies the encoder's codec context (any input source's audio
    /// stream will do). Publishes silence until inputs are added.
    pub async fn start(id: &str, publish_url: &str, template: AvStream) -> anyhow::Result<Self> {
        let mixer = DynamicMixerTask::new(SAMPLE_RATE);
        Self::start_mixer(id, publish_url, template, mixer, AudioProfile::default()).await
    }

    pub async fn start_program(
        id: &str,
        publish_url: &str,
        profile: AudioProfile,
    ) -> anyhow::Result<Self> {
        let template = AvStream::new(
            0,
            ffmpeg_next::codec::Parameters::new(),
            ffmpeg_next::Rational(1, 48_000),
            ffmpeg_next::Rational(0, 1),
        );
        Self::start_mixer(
            id,
            publish_url,
            template,
            DynamicMixerTask::new(SAMPLE_RATE),
            profile,
        )
        .await
    }

    pub async fn start_with_original(
        id: &str,
        program: &MixBus,
        original: RawFrameReceiver,
    ) -> anyhow::Result<Self> {
        let mixer = DynamicMixerTask::with_program(SAMPLE_RATE, program.mixer.subscribe_program());
        mixer.add_input("original", original, DEFAULT_VOLUME);
        let template = AvStream::new(
            0,
            ffmpeg_next::codec::Parameters::new(),
            ffmpeg_next::Rational(1, 48_000),
            ffmpeg_next::Rational(0, 1),
        );
        Self::start_mixer(id, "", template, mixer, program.profile.clone()).await
    }

    async fn start_mixer(
        id: &str,
        publish_url: &str,
        template: AvStream,
        mixer: DynamicMixerTask,
        profile: AudioProfile,
    ) -> anyhow::Result<Self> {
        profile.validate()?;
        let mixed_rx = mixer.subscribe();

        let encoder = Self::encoder_with_profile(&template, &profile)?;
        // Grab the muxer stream description before the encoder is moved into the task.
        let out_stream = encoder.output_stream(0);
        let enc_task = EncoderTask::new();
        let encoded = enc_task.shared_audio(out_stream.clone())?;
        // Live output: lossy (drop under back-pressure) rather than stall the mix.
        enc_task.start(encoder, mixed_rx, false).await;
        let packet_rx = enc_task.subscribe();

        let publish_cancel = CancellationToken::new();
        let publish_interrupt = Arc::new(AtomicBool::new(false));
        let publish_handle = if publish_url.is_empty() {
            None
        } else {
            let (handle, ready) = spawn_publish(
                id.to_string(),
                publish_url.to_string(),
                out_stream,
                packet_rx,
                publish_cancel.clone(),
                publish_interrupt.clone(),
            );
            match ready.await {
                Ok(Ok(())) => Some(handle),
                result => {
                    publish_cancel.cancel();
                    publish_interrupt.store(true, Ordering::Release);
                    enc_task.stop_and_join().await;
                    let _ = handle.await;
                    anyhow::bail!(
                        "混音总线发布失败：{}",
                        match result {
                            Ok(Err(e)) => e,
                            _ => "发布任务未完成初始化".to_string(),
                        }
                    );
                }
            }
        };
        mixer.start();

        Ok(Self {
            id: id.to_string(),
            publish_url: publish_url.to_string(),
            mixer,
            enc_task,
            publish_cancel,
            publish_handle: Mutex::new(publish_handle),
            encoded,
            profile,
            publish_interrupt,
        })
    }

    #[cfg(test)]
    fn new_encoder(template: &AvStream) -> anyhow::Result<Encoder> {
        Self::encoder_with_profile(template, &AudioProfile::default())
    }

    fn encoder_with_profile(
        template: &AvStream,
        profile: &AudioProfile,
    ) -> anyhow::Result<Encoder> {
        // The mixer generates PTS in output samples, independently of the
        // original source's clock (e.g. G.711 at 8kHz or MPEG-TS at 90kHz).
        let mixed_stream = AvStream::new(
            template.index(),
            template.parameters().clone(),
            ffmpeg_next::Rational(1, i32::try_from(SAMPLE_RATE)?),
            template.rate(),
        );
        let settings = AudioSettings {
            codec: Some(profile.encoder_name().to_string()),
            sample_rate: Some(profile.sample_rate),
            channels: Some(profile.channels),
            bitrate: Some(profile.bitrate),
            ..Default::default()
        };
        Encoder::new_audio(&mixed_stream, settings, None)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn publish_url(&self) -> &str {
        &self.publish_url
    }

    pub fn profile(&self) -> &AudioProfile {
        &self.profile
    }

    pub fn encoded(&self) -> SharedEncodedAudio {
        self.encoded.clone()
    }

    pub async fn shutdown(&self) {
        self.publish_cancel.cancel();
        self.publish_interrupt.store(true, Ordering::Release);
        self.mixer.stop_and_join().await;
        self.enc_task.stop_and_join().await;
        let handle = self
            .publish_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
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

    pub fn set_channel_map(
        &self,
        source_id: &str,
        mapping: crate::ChannelMap,
    ) -> anyhow::Result<()> {
        self.mixer.set_channel_map(source_id, mapping.value())
    }

    /// Snapshot of this bus's inputs for the API / persistence.
    pub fn inputs_snapshot(&self) -> Vec<InputSnapshot> {
        self.mixer
            .inputs()
            .into_iter()
            .map(|(source_id, volume, muted)| InputSnapshot {
                channel_map: crate::ChannelMap::from_value(self.mixer.channel_map(&source_id)),
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
        self.publish_interrupt.store(true, Ordering::Release);
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
    interrupt: Arc<AtomicBool>,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::oneshot::Receiver<Result<(), String>>,
) {
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::task::spawn_blocking(move || {
        let open = || -> anyhow::Result<AvOutput> {
            let mut options = ffmpeg_next::Dictionary::new();
            options.set("rw_timeout", "5000000");
            let mut output = AvOutput::open_network(
                &publish_url,
                Some("flv"),
                Some(options),
                interrupt.clone(),
            )?;
            output.add_stream(&out_stream)?;
            output.write_header()?;
            Ok(output)
        };
        let mut output = match open() {
            Ok(output) => output,
            Err(e) => {
                log::error!("audio bus '{id}' open output {publish_url}: {e:#}");
                let _ = ready_tx.send(Err(format!("{e:#}")));
                return;
            }
        };
        let _ = ready_tx.send(Ok(()));
        log::info!("audio bus '{id}' publishing -> {publish_url}");

        while !cancel.is_cancelled() {
            match packet_rx.blocking_recv() {
                Ok(RawPacketCmd::Data(pkt)) => {
                    if let Err(e) = output.write_packet(0, pkt) {
                        log::warn!("audio bus '{id}' write_packet: {e:#}");
                        drop(output);
                        loop {
                            if cancel.is_cancelled() {
                                return;
                            }
                            for _ in 0..20 {
                                if cancel.is_cancelled() {
                                    return;
                                }
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                            match open() {
                                Ok(fresh) => {
                                    output = fresh;
                                    break;
                                }
                                Err(e) => log::warn!("audio bus '{id}' reconnect: {e:#}"),
                            }
                        }
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
    (handle, ready_rx)
}

#[cfg(test)]
#[path = "bus_test.rs"]
mod bus_test;
