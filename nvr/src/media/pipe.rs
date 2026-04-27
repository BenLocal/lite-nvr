use std::{
    backtrace::Backtrace,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use ffmpeg_bus::bus::{Bus as FbBus, OutputAvType, VideoRawFrameStream};
use futures::StreamExt;
#[cfg(feature = "zlm")]
use rszlm::{
    frame::Frame as ZlmFrame,
    obj::{AudioCodecArgs, CodecArgs, CodecId, Track, VideoCodecArgs},
};
#[cfg(feature = "zlm")]
use std::sync::Mutex as SyncMutex;
#[cfg(feature = "zlm")]
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::media::{
    stream::RawSinkSource,
    types::{EncodeConfig, InputConfig, OutputConfig, OutputDest, PipeConfig, VideoRawFrame},
};

/// Pipeline: media processing using ffmpeg-bus
pub struct Pipe {
    config: PipeConfig,
    cancel: CancellationToken,
    started: AtomicBool,
}

impl Pipe {
    pub fn new(config: PipeConfig) -> Self {
        Self {
            config,
            cancel: CancellationToken::new(),
            started: AtomicBool::new(false),
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Check if the pipeline has been started
    pub fn is_started(&self) -> bool {
        self.started.load(Ordering::Relaxed)
    }

    /// Check if the pipeline has been cancelled
    #[allow(dead_code)]
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Start the pipeline
    pub async fn start(&self) {
        if self.started.swap(true, Ordering::Relaxed) {
            log::warn!("Pipe already started");
            return;
        }

        let log_input = match &self.config.input {
            InputConfig::Network { url } => format!("net://{}", url),
            InputConfig::File { path } => format!("file://{}", path),
            InputConfig::Device { display, format } => format!("device://{} ({})", display, format),
        };

        log::info!("Pipe: starting with input {}", log_input);

        let bus = FbBus::new("pipe");
        let cancel = self.cancel.clone();

        // Map and add input
        let fb_input = self.config.input.clone().into();
        if let Err(e) = bus.add_input(fb_input, None).await {
            log::error!(
                "Pipe: add_input failed: {:#}\nbacktrace:\n{}",
                e,
                Backtrace::capture()
            );
            self.started.store(false, Ordering::Relaxed);
            return;
        }

        // First pass: register all outputs with the bus; collect successes.
        // We hold the streams temporarily so we can size the ZLM track coordinator
        // based on successful Zlm outputs (an audio output may fail if the input
        // has no audio stream).
        let mut accepted: Vec<(usize, ffmpeg_bus::stream::AvStream, VideoRawFrameStream, OutputConfig)> =
            Vec::new();
        for (i, output_config) in self.config.outputs.iter().enumerate() {
            let id = format!("out_{}", i);
            let fb_output = match output_config.clone().into() {
                Some(o) => o,
                None => {
                    log::warn!(
                        "Pipe: skip unsupported output {:?}",
                        dest_name(&output_config.dest)
                    );
                    continue;
                }
            };
            match bus.add_output(fb_output).await {
                Ok((av, stream)) => {
                    accepted.push((i, av, stream, output_config.clone()));
                }
                Err(e) => {
                    log::warn!("Pipe: add_output {} failed: {:#}", id, e);
                }
            }
        }

        // Build per-Media ZLM track coordinators (each Pipe drives at most one
        // ZLM Media, but we group by Arc identity to be safe).
        #[cfg(feature = "zlm")]
        let zlm_coordinator: Option<Arc<ZlmTrackCoordinator>> = {
            let mut media_ref: Option<Arc<rszlm::media::Media>> = None;
            let mut count = 0usize;
            for (_, _, _, oc) in accepted.iter() {
                if let OutputDest::Zlm(m) = &oc.dest {
                    if media_ref.is_none() {
                        media_ref = Some(Arc::clone(m));
                    }
                    count += 1;
                }
            }
            media_ref.map(|m| ZlmTrackCoordinator::new(m, count))
        };

        // Second pass: spawn forwarder tasks.
        let mut join_handles = Vec::new();
        for (_, av, stream, output_config) in accepted {
            match &output_config.dest {
                OutputDest::RawFrame { sink } => {
                    let sink = Arc::clone(sink);
                    let handle = tokio::spawn(async move {
                        forward_frame_stream_to_sink(stream, sink).await;
                    });
                    join_handles.push(handle);
                }
                OutputDest::RawPacket { sink } => {
                    let sink = Arc::clone(sink);
                    let handle = tokio::spawn(async move {
                        forward_frame_stream_to_sink(stream, sink).await;
                    });
                    join_handles.push(handle);
                }
                #[cfg(feature = "zlm")]
                OutputDest::Zlm(media) => {
                    let media = Arc::clone(media);
                    let coord = zlm_coordinator.as_ref().map(Arc::clone);
                    let av_type = output_config.av_type;
                    let handle = tokio::spawn(async move {
                        forward_raw_packet_stream_to_zlm(stream, av, media, coord, av_type).await;
                    });
                    join_handles.push(handle);
                }
                OutputDest::Network { .. } => {}
            }
        }

        if join_handles.is_empty() && !self.config.outputs.is_empty() {
            log::warn!("Pipe: no output task running");
        }

        // Wait for cancellation
        tokio::select! {
            _ = cancel.cancelled() => {
                log::info!("Pipe: cancelled");
            }
        }

        // Stop input and outputs: remove input first so the bus stops feeding streams
        if let Err(e) = bus.remove_input().await {
            log::warn!("Pipe: remove_input failed: {:#}", e);
        }
        bus.stop();
        for h in join_handles {
            let _ = h.await;
        }

        self.started.store(false, Ordering::Relaxed);
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Forwards ffmpeg-bus VideoFrame stream to nvr RawSinkSource (VideoRawFrame).
async fn forward_frame_stream_to_sink(
    mut stream: ffmpeg_bus::bus::VideoRawFrameStream,
    sink: Arc<RawSinkSource>,
) {
    while let Some(opt) = stream.next().await {
        if let Some(frame) = opt {
            let vf = VideoRawFrame::new(
                frame.data.to_vec(),
                frame.width,
                frame.height,
                frame.format,
                frame.pts,
                frame.dts,
                frame.is_key,
                frame.codec_id,
            );
            if sink.writer.try_send(vf).is_err() {
                break;
            }
        }
    }
}

/// Coordinator that batches `init_track` calls across multiple ZLM forwarders
/// (e.g. video + audio) and triggers `init_complete` once all expected tracks
/// have been registered. Frame forwarders await this barrier before pushing
/// any frame so ZLM has the full SDP ready.
#[cfg(feature = "zlm")]
struct ZlmTrackCoordinator {
    media: Arc<rszlm::media::Media>,
    expected: usize,
    registered: SyncMutex<usize>,
    completed_tx: watch::Sender<bool>,
}

#[cfg(feature = "zlm")]
impl ZlmTrackCoordinator {
    fn new(media: Arc<rszlm::media::Media>, expected: usize) -> Arc<Self> {
        let (tx, _rx) = watch::channel(false);
        Arc::new(Self {
            media,
            expected,
            registered: SyncMutex::new(0),
            completed_tx: tx,
        })
    }

    /// Register `track` synchronously and return a watcher for the
    /// `init_complete` signal. `Track` is non-`Send`, so it's consumed entirely
    /// in this sync function, never crossing an await point.
    fn register_track(&self, track: Track) -> watch::Receiver<bool> {
        let rx = self.completed_tx.subscribe();
        let mut count = self.registered.lock().expect("track count mutex poisoned");
        self.media.init_track(&track);
        *count += 1;
        if *count >= self.expected {
            self.media.init_complete();
            let _ = self.completed_tx.send(true);
        }
        rx
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

/// Forward raw (demuxed) packet stream from ffmpeg-bus to ZLMediaKit Media.
/// The Mux output with format "h264"/"aac" uses a large buffer (256KB) so each
/// chunk is a complete NALU (Annex B) or AAC ADTS frame. PTS/DTS are converted to ms.
/// H.264/H.265 from MP4 (AVCC/HVCC) is auto-converted to Annex B if needed.
///
/// Track init is gated by [`ZlmTrackCoordinator`] so video + audio register
/// together before `init_complete()` is invoked.
#[cfg(feature = "zlm")]
async fn forward_raw_packet_stream_to_zlm(
    mut stream: VideoRawFrameStream,
    av: ffmpeg_bus::stream::AvStream,
    media: Arc<rszlm::media::Media>,
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
    let sample_rate = av.sample_rate();
    let channels = av.channels();
    let mut track_initialized = false;
    let mut needs_conversion = false;
    let mut conversion_checked = false;

    // ADTS-framed AAC may arrive with multiple frames concatenated in one chunk
    // (the ADTS muxer's avio buffer batches small writes). ZLM expects a single
    // AAC frame per `mk_frame`, so we walk the chunk and emit one ZlmFrame per
    // ADTS frame. Partial frames at chunk boundaries are buffered.
    let mut adts_leftover: Vec<u8> = Vec::new();
    let pts_step_ms_per_aac_frame: u64 = if sample_rate > 0 {
        ((1024u64 * 1000) + sample_rate as u64 / 2) / sample_rate as u64 // ≈21ms@48k, ≈23ms@44.1k
    } else {
        21
    };

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
                        log::info!(
                            "ZLM: video track init ({}x{}, fps={})",
                            w,
                            h,
                            default_fps
                        );
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
                        let sr = sample_rate.max(1) as i32;
                        let ch = channels.max(1) as i32;
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

        if matches!(av_type, OutputAvType::Audio) {
            // Combine any leftover from prior chunk with this chunk, split
            // ADTS frames, and push each as a separate ZlmFrame.
            let combined: Vec<u8> = if adts_leftover.is_empty() {
                frame.data.to_vec()
            } else {
                let mut v = std::mem::take(&mut adts_leftover);
                v.extend_from_slice(frame.data.as_ref());
                v
            };
            let mut offset = 0usize;
            let mut sub_idx = 0u64;
            while offset + 7 <= combined.len() {
                let Some(len) = parse_adts_frame_len(&combined[offset..]) else {
                    // Lost sync — drop one byte and resync.
                    offset += 1;
                    continue;
                };
                if len < 7 || offset + len > combined.len() {
                    break;
                }
                let frame_bytes = &combined[offset..offset + len];
                let base_pts = pts_ms.max(0.0) as u64;
                let pts = base_pts.saturating_add(pts_step_ms_per_aac_frame * sub_idx);
                let zlm_frame = ZlmFrame::new(CodecId::AAC, pts, pts, frame_bytes);
                if !media.input_frame(&zlm_frame) {
                    log::warn!(
                        "ZLM: input_frame failed (audio, pts_ms={}, len={})",
                        pts,
                        frame_bytes.len()
                    );
                }
                offset += len;
                sub_idx += 1;
            }
            if offset < combined.len() {
                adts_leftover.extend_from_slice(&combined[offset..]);
            }
            continue;
        }

        let data: std::borrow::Cow<'_, [u8]> =
            if matches!(av_type, OutputAvType::Video) && needs_conversion {
                std::borrow::Cow::Owned(convert_avcc_to_annexb(frame.data.as_ref()).to_vec())
            } else {
                std::borrow::Cow::Borrowed(frame.data.as_ref())
            };

        let zlm_frame =
            ZlmFrame::new(make_codec_id(), dts_ms as u64, pts_ms as u64, data.as_ref());
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

/// Parse the frame length (header + payload) from an ADTS frame. Returns
/// `None` if the bytes don't start with a valid ADTS sync word (0xFFF).
#[cfg(feature = "zlm")]
fn parse_adts_frame_len(buf: &[u8]) -> Option<usize> {
    if buf.len() < 7 {
        return None;
    }
    if buf[0] != 0xFF || (buf[1] & 0xF0) != 0xF0 {
        return None;
    }
    let len = ((buf[3] as usize & 0x03) << 11)
        | ((buf[4] as usize) << 3)
        | (((buf[5] as usize) >> 5) & 0x07);
    Some(len)
}

/// Get destination name for logging (used by tests).
pub fn dest_name(dest: &OutputDest) -> String {
    match dest {
        OutputDest::Network { url, .. } => url.clone(),
        OutputDest::RawFrame { .. } => "RawFrame".to_string(),
        OutputDest::RawPacket { .. } => "RawPacket".to_string(),
        #[cfg(feature = "zlm")]
        OutputDest::Zlm(_) => "Zlm".to_string(),
    }
}

impl PipeConfig {
    #[allow(dead_code)]
    pub fn builder() -> PipeConfigBuilder {
        PipeConfigBuilder::default()
    }
}

#[allow(dead_code)]
#[derive(Default)]
pub struct PipeConfigBuilder {
    input: Option<InputConfig>,
    outputs: Vec<OutputConfig>,
}

#[allow(dead_code)]
impl PipeConfigBuilder {
    /// Set network input source
    pub fn input_url(mut self, url: impl Into<String>) -> Self {
        self.input = Some(InputConfig::Network { url: url.into() });
        self
    }

    /// Set file input source
    pub fn input_file(mut self, path: impl Into<String>) -> Self {
        self.input = Some(InputConfig::File { path: path.into() });
        self
    }

    /// Add RTSP output
    /// if encode is None, the output will be remuxed
    /// if encode is Some, the output will be encoded
    pub fn add_rtsp_output(mut self, url: impl Into<String>, encode: Option<EncodeConfig>) -> Self {
        self.outputs.push(OutputConfig::new(
            OutputDest::Network {
                url: url.into(),
                format: "rtsp".to_string(),
            },
            encode,
        ));

        self
    }

    /// Add direct remux output (no re-encoding)
    pub fn add_remux_output(mut self, url: impl Into<String>, format: impl Into<String>) -> Self {
        self.outputs.push(OutputConfig::new(
            OutputDest::Network {
                url: url.into(),
                format: format.into(),
            },
            None,
        ));
        self
    }

    /// Add raw frame output
    pub fn add_raw_frame_output(mut self, sink: Arc<RawSinkSource>) -> Self {
        self.outputs
            .push(OutputConfig::new(OutputDest::RawFrame { sink }, None));
        self
    }

    /// Add encoded packet output
    pub fn add_raw_packet_output(mut self, sink: Arc<RawSinkSource>, encode: EncodeConfig) -> Self {
        self.outputs.push(OutputConfig::new(
            OutputDest::RawPacket { sink },
            Some(encode),
        ));
        self
    }

    /// Add zlm output
    #[cfg(feature = "zlm")]
    pub fn add_zlm_output(mut self, media: Arc<rszlm::media::Media>) -> Self {
        self.outputs
            .push(OutputConfig::new(OutputDest::Zlm(media), None));
        self
    }

    pub fn build(self) -> PipeConfig {
        PipeConfig {
            input: self.input.expect("input is required"),
            outputs: self.outputs,
        }
    }
}

#[cfg(test)]
#[path = "pipe_test.rs"]
mod pipe_test;
