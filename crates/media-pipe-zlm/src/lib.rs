//! ZLMediaKit sink for [`media-pipe-core`].
//!
//! Implements [`DemuxedSink`] by forwarding a pipe's demuxed packet stream into
//! a ZLMediaKit `Media` as `Track`/`Frame`s. Video + audio tracks of one
//! `Media` are gated by a shared [`ZlmTrackCoordinator`] so `init_complete()` is
//! only called once both sides have registered.

use std::sync::{Arc, Mutex as SyncMutex};

use ffmpeg_bus::bus::{OutputAvType, VideoRawFrameStream};
use ffmpeg_bus::stream::AvStream;
use futures::StreamExt;
use media_pipe_core::{DemuxedSink, OutputConfig, OutputDest};
use rszlm::{
    frame::Frame as ZlmFrame,
    media::Media,
    obj::{AudioCodecArgs, CodecArgs, CodecId, Track, VideoCodecArgs},
};
use tokio::{sync::watch, task::JoinHandle};

/// A [`DemuxedSink`] that forwards demuxed packets to a ZLMediaKit `Media`.
pub struct ZlmSink {
    media: Arc<Media>,
    coordinator: Option<Arc<ZlmTrackCoordinator>>,
    av_type: OutputAvType,
}

impl ZlmSink {
    pub fn new(
        media: Arc<Media>,
        coordinator: Option<Arc<ZlmTrackCoordinator>>,
        av_type: OutputAvType,
    ) -> Self {
        Self {
            media,
            coordinator,
            av_type,
        }
    }
}

impl DemuxedSink for ZlmSink {
    fn start(&self, av: AvStream, stream: VideoRawFrameStream) -> JoinHandle<()> {
        let media = Arc::clone(&self.media);
        let coordinator = self.coordinator.clone();
        let av_type = self.av_type;
        tokio::spawn(async move {
            forward_raw_packet_stream_to_zlm(stream, av, media, coordinator, av_type).await;
        })
    }

    fn on_rejected(&self) {
        // The Pipe could not create this output (e.g. no audio in the input);
        // drop it from the coordinator's expected set so the surviving track(s)
        // can still finalize.
        if let Some(coord) = &self.coordinator {
            coord.expect_one_less();
        }
    }
}

/// Convenience: build the ZLM outputs for one `Media` — a video track plus an
/// optional audio track, sharing a coordinator. Mirrors the device pipeline.
pub fn zlm_outputs(media: Arc<Media>, include_audio: bool) -> Vec<OutputConfig> {
    let expected = if include_audio { 2 } else { 1 };
    let coordinator = ZlmTrackCoordinator::new(Arc::clone(&media), expected);
    let mut outs = vec![OutputConfig::new(
        OutputDest::Demuxed {
            sink: Arc::new(ZlmSink::new(
                Arc::clone(&media),
                Some(Arc::clone(&coordinator)),
                OutputAvType::Video,
            )),
        },
        None,
    )];
    if include_audio {
        outs.push(
            OutputConfig::new(
                OutputDest::Demuxed {
                    sink: Arc::new(ZlmSink::new(media, Some(coordinator), OutputAvType::Audio)),
                },
                None,
            )
            .with_av_type(OutputAvType::Audio),
        );
    }
    outs
}

/// Convenience: a single video `Demuxed` destination for one `Media` (no audio),
/// for callers that build the `OutputConfig` themselves (e.g. with an encode).
pub fn zlm_video_dest(media: Arc<Media>) -> OutputDest {
    let coordinator = ZlmTrackCoordinator::new(Arc::clone(&media), 1);
    OutputDest::Demuxed {
        sink: Arc::new(ZlmSink::new(media, Some(coordinator), OutputAvType::Video)),
    }
}

struct CoordState {
    expected: usize,
    registered: usize,
    completed: bool,
}

/// Batches `init_track` calls across multiple ZLM forwarders (e.g. video +
/// audio) and triggers `init_complete` once all expected tracks have registered.
/// Frame forwarders await this barrier before pushing any frame so ZLM has the
/// full SDP ready.
pub struct ZlmTrackCoordinator {
    media: Arc<Media>,
    state: SyncMutex<CoordState>,
    completed_tx: watch::Sender<bool>,
}

impl ZlmTrackCoordinator {
    pub fn new(media: Arc<Media>, expected: usize) -> Arc<Self> {
        let (tx, _rx) = watch::channel(false);
        Arc::new(Self {
            media,
            state: SyncMutex::new(CoordState {
                expected,
                registered: 0,
                completed: false,
            }),
            completed_tx: tx,
        })
    }

    fn try_finalize(&self, state: &mut CoordState) {
        if !state.completed && state.registered > 0 && state.registered >= state.expected {
            self.media.init_complete();
            let _ = self.completed_tx.send(true);
            state.completed = true;
        }
    }

    /// Register `track` synchronously and return a watcher for the
    /// `init_complete` signal. `Track` is non-`Send`, so it's consumed entirely
    /// in this sync function, never crossing an await point.
    fn register_track(&self, track: Track) -> watch::Receiver<bool> {
        let rx = self.completed_tx.subscribe();
        let mut state = self.state.lock().expect("track count mutex poisoned");
        self.media.init_track(&track);
        state.registered += 1;
        self.try_finalize(&mut state);
        rx
    }

    /// Drop one track from the expected set (its output was rejected by the
    /// Pipe), finalizing if the surviving tracks have all registered.
    fn expect_one_less(&self) {
        let mut state = self.state.lock().expect("track count mutex poisoned");
        state.expected = state.expected.saturating_sub(1);
        self.try_finalize(&mut state);
    }

    async fn wait_complete(rx: &mut watch::Receiver<bool>) {
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}

/// Forward a raw (demuxed) packet stream from ffmpeg-bus to a ZLMediaKit Media.
/// Each emitted item is one raw codec frame — for audio one AAC frame (no ADTS
/// header), for video a NALU group in Annex B (or AVCC, converted below). PTS/DTS
/// are converted to ms. Track init is gated by [`ZlmTrackCoordinator`].
async fn forward_raw_packet_stream_to_zlm(
    mut stream: VideoRawFrameStream,
    av: AvStream,
    media: Arc<Media>,
    coordinator: Option<Arc<ZlmTrackCoordinator>>,
    av_type: OutputAvType,
) {
    use ffmpeg_bus::bsf::{convert_avcc_to_annexb, is_annexb_packet};

    let make_codec_id = || match av_type {
        OutputAvType::Video => CodecId::H264,
        OutputAvType::Audio => CodecId::AAC,
    };

    let default_width = av.width();
    let default_height = av.height();
    let default_fps = av.fps();
    let audio_sample_rate = av.sample_rate();
    let audio_channels = av.channels();
    let mut track_initialized = false;
    let mut needs_conversion = false;
    let mut conversion_checked = false;

    while let Some(opt) = stream.next().await {
        let Some(frame) = opt else { continue };

        if !track_initialized {
            // Build + register Track in a sync block so the non-`Send` Track is
            // dropped before any `.await` (track holds a raw FFI pointer).
            let mut completion_rx = {
                let track = match av_type {
                    OutputAvType::Video => {
                        let w = if frame.width > 0 {
                            frame.width as i32
                        } else {
                            default_width as i32
                        };
                        let h = if frame.height > 0 {
                            frame.height as i32
                        } else {
                            default_height as i32
                        };
                        log::info!("ZLM: video track init ({}x{}, fps={})", w, h, default_fps);
                        Track::new(
                            CodecId::H264,
                            Some(CodecArgs::Video(VideoCodecArgs {
                                width: w,
                                height: h,
                                fps: default_fps,
                            })),
                        )
                    }
                    OutputAvType::Audio => {
                        let sr = audio_sample_rate.max(1) as i32;
                        let ch = audio_channels.max(1) as i32;
                        log::info!("ZLM: audio track init (sr={}, ch={})", sr, ch);
                        Track::new(
                            CodecId::AAC,
                            Some(CodecArgs::Audio(AudioCodecArgs {
                                sample_rate: sr,
                                channels: ch,
                            })),
                        )
                    }
                };
                if let Some(ref coord) = coordinator {
                    Some(coord.register_track(track))
                } else {
                    media.init_track(&track);
                    media.init_complete();
                    None
                }
                // `track` dropped here.
            };
            if let Some(ref mut rx) = completion_rx {
                ZlmTrackCoordinator::wait_complete(rx).await;
            }
            track_initialized = true;

            if matches!(av_type, OutputAvType::Video) && !conversion_checked {
                needs_conversion = !is_annexb_packet(frame.data.as_ref());
                conversion_checked = true;
                log::info!(
                    "ZLM: video format {}",
                    if needs_conversion {
                        "MP4 (AVCC) — BSF conversion enabled"
                    } else {
                        "Annex B — no conversion"
                    }
                );
            }
        }

        let time_base = av.time_base();
        let pts_ms = frame.pts_ms(time_base);
        let dts_ms = frame.dts_ms(time_base);

        let data: std::borrow::Cow<'_, [u8]> =
            if matches!(av_type, OutputAvType::Video) && needs_conversion {
                std::borrow::Cow::Owned(convert_avcc_to_annexb(frame.data.as_ref()).to_vec())
            } else {
                std::borrow::Cow::Borrowed(frame.data.as_ref())
            };

        let zlm_frame = ZlmFrame::new(make_codec_id(), dts_ms as u64, pts_ms as u64, data.as_ref());
        if !media.input_frame(&zlm_frame) {
            log::warn!(
                "ZLM: input_frame failed (av={:?}, pts_ms={}, dts_ms={}, len={})",
                av_type,
                pts_ms,
                dts_ms,
                data.len()
            );
        }
    }

    log::info!("ZLM: {:?} stream ended", av_type);
}
