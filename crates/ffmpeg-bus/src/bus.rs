use std::{
    backtrace::Backtrace,
    collections::{HashMap, HashSet},
    hash::Hasher,
    pin::Pin,
    sync::Arc,
};

use futures::{Stream, StreamExt};
use log::error;
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;

use ffmpeg_next::Dictionary;

use crate::{
    decoder::{Decoder, DecoderTask},
    encoder::{AudioSettings, Encoder, EncoderTask, Settings, pixel_format_for_libx264},
    frame::{RawFrameCmd, VideoFrame, packet_to_raw_video_frame},
    input::{AvInput, AvInputTask},
    output::{AvOutput, AvOutputStream},
    packet::{RawPacket, RawPacketCmd, RawPacketReceiver},
    stream::AvStream,
};

/// Destination for the multi-stream muxer.
enum MuxTarget {
    File(String),
    Net { url: String, format: Option<String> },
}

/// An item flowing into the multi-stream muxer: a packet for a given output
/// stream index, or the end-of-stream signal for one source.
enum MuxSignal {
    Packet(usize, RawPacket),
    /// Packets of these output streams were lost (the receiver lagged).
    Gap(Vec<usize>),
    Eof,
}

/// Per-stream resync after data loss. Once a stream has a gap (lost
/// packets), its packets are dropped until its next keyframe: writing the
/// frames after a hole would record corrupted video until then anyway.
#[derive(Default)]
struct KeyframeGate {
    waiting: HashSet<usize>,
    /// All packets not written (no room, or waiting for a keyframe).
    dropped: u64,
    /// Packets dropped for lack of room; paces the log lines.
    overflowed: u64,
}

impl KeyframeGate {
    fn mark_gap(&mut self, index: usize) {
        self.waiting.insert(index);
    }

    /// Whether to write this packet; reopens a stream on its keyframe.
    fn admit(&mut self, index: usize, is_key: bool) -> bool {
        if self.waiting.contains(&index) {
            if !is_key {
                self.dropped += 1;
                return false;
            }
            self.waiting.remove(&index);
        }
        true
    }

    /// Count a packet dropped for lack of room; returns true when it is worth
    /// a log line (first drop, then every 100).
    fn note_drop(&mut self, index: usize) -> bool {
        self.mark_gap(index);
        self.dropped += 1;
        self.overflowed += 1;
        self.overflowed % 100 == 1
    }
}

/// Codec a raw Mux format carries, for formats an encoder can feed.
fn mux_format_codec(format: &str) -> Option<ffmpeg_next::codec::Id> {
    use ffmpeg_next::codec::Id;
    Some(match format {
        "h264" => Id::H264,
        "hevc" | "h265" => Id::HEVC,
        "aac" | "adts" => Id::AAC,
        "opus" => Id::OPUS,
        _ => return None,
    })
}

/// Default encoder for a Mux format when the output has no encode config.
fn mux_format_encode(format: &str) -> Option<EncodeConfig> {
    let codec = match format {
        "h264" => "h264",
        "hevc" | "h265" => "hevc",
        "aac" | "adts" => "aac",
        // FFmpeg's native opus encoder is experimental; libopus is the norm.
        "opus" => "libopus",
        _ => return None,
    };
    Some(EncodeConfig {
        codec: codec.to_string(),
        ..Default::default()
    })
}

/// Pump packets from `rx` into an in-memory muxer until EOF, the source
/// closing, or the reader going away. Blocking (the muxer's write callback
/// waits for the reader instead of dropping bytes): run on a blocking thread.
/// `index` is the muxed stream's index; with `retag` every packet is
/// re-indexed to it (encoder packets), otherwise packets of other streams
/// are skipped (demuxed input). After a gap, resumes on a keyframe.
fn run_mux_stream_writer(
    mut writer: crate::output::AvOutputStreamWriter,
    mut rx: RawPacketReceiver,
    index: usize,
    retag: bool,
) {
    let mut gate = KeyframeGate::default();
    loop {
        match rx.blocking_recv() {
            Ok(RawPacketCmd::Data(mut packet)) => {
                // Reader dropped: stop, releasing the source.
                if writer.is_closed() {
                    break;
                }
                if retag {
                    packet.get_mut().set_stream(index);
                } else if packet.index() != index {
                    continue;
                }
                if !gate.admit(index, packet.is_key()) {
                    continue;
                }
                if let Err(e) = writer.write_packet(packet) {
                    log::error!("mux write_packet error: {e:#}");
                }
            }
            Ok(RawPacketCmd::EOF) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                log::warn!("mux source lagged, lost {n} packets; resuming at a keyframe");
                gate.mark_gap(index);
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
    if let Err(e) = writer.finish() {
        log::error!(
            "mux finish error: {:#?}\nbacktrace:\n{}",
            e,
            Backtrace::capture()
        );
    }
    log::info!("mux stream finished");
}

/// Run a codec open on a blocking thread so slow hardware probing never
/// stalls the bus's async command loop.
async fn open_blocking<T: Send + 'static>(
    open: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(open)
        .await
        .map_err(|e| anyhow::anyhow!("codec open task failed: {e}"))?
}

/// Options for a network output: RTSP publishes over TCP with its socket
/// timeout; every other protocol gets the generic AVIO `rw_timeout`.
fn net_output_options(format: Option<&str>) -> Dictionary<'static> {
    let mut opts = Dictionary::new();
    if format == Some("rtsp") {
        // TCP interleaving is the reliable choice for publishing; the timeout
        // bounds a server that accepts but never answers.
        opts.set("rtsp_transport", "tcp");
        opts.set("timeout", DEFAULT_RTSP_TIMEOUT_US);
    } else {
        opts.set("rw_timeout", DEFAULT_RTSP_TIMEOUT_US);
    }
    opts
}

/// A write failure that means the network output is gone for good (timeout,
/// peer reset, broken pipe, interrupted): stop writing instead of failing
/// every following packet the same way.
fn is_connection_error(e: &anyhow::Error) -> bool {
    match e.downcast_ref::<ffmpeg_next::Error>() {
        Some(ffmpeg_next::Error::Exit) => true,
        Some(ffmpeg_next::Error::Other { errno }) => [
            ffmpeg_next::util::error::ETIMEDOUT,
            ffmpeg_next::util::error::EPIPE,
            ffmpeg_next::util::error::ECONNRESET,
            ffmpeg_next::util::error::EIO,
        ]
        .contains(errno),
        _ => false,
    }
}

/// Whether an input is a live source when the caller does not say: files are
/// not; network streams and capture devices are.
fn infer_live(input: &InputConfig) -> bool {
    !matches!(input, InputConfig::File { .. })
}

/// RTSP socket I/O timeout (microseconds) used when the caller sets none, so
/// an unresponsive camera or server fails instead of blocking forever.
const DEFAULT_RTSP_TIMEOUT_US: &str = "10000000";

/// Packets queued between a File/Net mux's merge task and its writer thread.
/// Bounded so a slow disk / network backpressures the sources.
const MUX_WRITE_QUEUE: usize = 64;

fn is_rtsp_url(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.starts_with("rtsp://") || url.starts_with("rtsps://")
}

/// Input options with the default RTSP timeout added for RTSP client inputs
/// (not listen mode, which waits for a publisher by design).
fn rtsp_input_options(
    url: &str,
    options: Option<HashMap<String, String>>,
) -> Option<HashMap<String, String>> {
    if !is_rtsp_url(url) {
        return options;
    }
    let mut opts = options.unwrap_or_default();
    let listening = opts.get("rtsp_flags").is_some_and(|f| f.contains("listen"));
    if !listening {
        opts.entry("timeout".to_string())
            .or_insert_with(|| DEFAULT_RTSP_TIMEOUT_US.to_string());
    }
    Some(opts)
}

/// One stream's role in a File/Net mux: copy the demuxed input through, or
/// transcode it via its encoder task.
struct MuxPlanEntry {
    input_index: usize,
    transcode: bool,
    /// Encode config when transcoding (used to start the encoder task).
    encode: Option<EncodeConfig>,
    /// Target codec id for the muxed output stream.
    codec_id: ffmpeg_next::codec::Id,
    /// Encoder serving this stream, set once its task starts (transcode only).
    encoder_key: Option<EncoderKey>,
}

/// Identity of a shared encoder. Outputs with the same stream, encode config
/// and loss policy share one encoder; any difference starts another, so one
/// output's config is never silently applied to another.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct EncoderKey {
    stream_index: usize,
    encode: Option<EncodeConfig>,
    lossless: bool,
}

pub struct Bus {
    id: String,
    cancel: CancellationToken,
    tx: tokio::sync::mpsc::Sender<BusCommand>,
    /// Set by [`Bus::stop`]: aborts an input open in progress (the command
    /// loop is busy awaiting it, so cancellation alone cannot reach it).
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}

impl Bus {
    /// A bus that starts reading its input as soon as the first output (or
    /// subscription) needs it.
    pub fn new(id: &str) -> Self {
        Self::spawn(id, false)
    }

    /// A bus that only starts reading its input on [`Bus::start`]. Register
    /// every output first, then start: with a fast source (e.g. a short file)
    /// an output added after reading began would miss the start, or even the
    /// end, of the stream.
    pub fn new_deferred(id: &str) -> Self {
        Self::spawn(id, true)
    }

    fn spawn(id: &str, deferred: bool) -> Self {
        let id = id.to_string();
        let cancel = CancellationToken::new();
        let (tx, rx) = tokio::sync::mpsc::channel(1024);

        let cancel_clone = cancel.clone();
        let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();
        tokio::spawn(
            async move { Self::inner_loop(cancel_clone, rx, deferred, shutdown_clone).await },
        );
        Self {
            id: id,
            cancel,
            tx,
            shutdown,
        }
    }

    async fn inner_loop(
        cancel: CancellationToken,
        mut rx: tokio::sync::mpsc::Receiver<BusCommand>,
        deferred: bool,
        shutdown: Arc<std::sync::atomic::AtomicBool>,
    ) {
        let cancel_clone = cancel.clone();
        let mut state = BusState::new();
        state.deferred = deferred;
        state.shutdown = shutdown;
        loop {
            tokio::select! {
                _ = cancel_clone.cancelled() => {
                    break;
                },
                Some(cmd) = rx.recv() => {
                    if let Err(e) = Self::inner_command_handler(&mut state, cmd).await {
                        error!("inner_command_handler error: {:#?}\nbacktrace:\n{}", e, Backtrace::capture());
                    }
                },
            }
        }
    }

    async fn inner_command_handler(state: &mut BusState, cmd: BusCommand) -> anyhow::Result<()> {
        match cmd {
            BusCommand::AddInput {
                input,
                options,
                live,
                result,
            } => {
                result
                    .send(Self::add_input_internal(state, input, options, live).await)
                    .map_err(|e| anyhow::anyhow!("send result error: {:#?}", e))?;
            }
            BusCommand::RemoveInput { result } => {
                if let Some(input) = state.input_task.take() {
                    input.stop();
                    drop(input);
                }
                state.pending_input = None;
                state.input_config = None;
                // Everything below was derived from the removed input; dropping
                // the tasks cancels them (no leaked relay / blocking threads),
                // and a later AddInput starts from a clean state.
                state.decoder_tasks.clear();
                state.encoder_tasks.clear();
                state.encoder_output_streams.clear();
                state.input_streams.clear();
                state.output_config.clear();
                // A deferred bus waits for `start()` again on its next input.
                state.started = false;
                result
                    .send(Ok(()))
                    .map_err(|e| anyhow::anyhow!("send result error: {:#?}", e))?;
            }
            BusCommand::AddOutput { output, result } => {
                let r = Self::add_output_internal(state, output).await;
                let err = r.as_ref().err().map(|e| format!("{e:#}"));
                // Always reply (with the real error), even if the caller has
                // stopped waiting; then log the failure via the loop.
                let _ = result.send(r);
                if let Some(msg) = err {
                    return Err(anyhow::anyhow!(msg));
                }
            }
            BusCommand::Start { result } => {
                let r = Self::start_internal(state).await;
                let _ = result.send(r);
            }
            BusCommand::SubscribeAudio { result } => {
                let r = Self::subscribe_audio_internal(state).await;
                let _ = result.send(r);
            }
            BusCommand::SubscribeVideo { result } => {
                let r = Self::subscribe_video_internal(state).await;
                let _ = result.send(r);
            }
        }

        Ok(())
    }

    /// Register one output: start (or reuse) the decoder/encoder it needs and
    /// build its stream. On failure, tear down only the shared tasks this call
    /// started, so a rejected output leaves nothing running behind.
    async fn add_output_internal(
        state: &mut BusState,
        output: OutputConfig,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        if state.output_config.contains_key(&output.id) {
            anyhow::bail!("output already exists");
        }
        if state.input_task.is_none() && state.input_config.is_some() {
            Self::prepare_input_task(state).await?;
        }
        let decoders_before: HashSet<usize> = state.decoder_tasks.keys().copied().collect();
        let encoders_before: HashSet<EncoderKey> = state.encoder_tasks.keys().cloned().collect();
        match Self::build_output(state, &output).await {
            Ok(built) => {
                state.output_config.insert(output.id.clone(), output);
                Self::auto_start_input(state).await?;
                Ok(built)
            }
            Err(e) => {
                state
                    .decoder_tasks
                    .retain(|k, _| decoders_before.contains(k));
                state
                    .encoder_tasks
                    .retain(|k, _| encoders_before.contains(k));
                state
                    .encoder_output_streams
                    .retain(|k, _| encoders_before.contains(k));
                Err(e)
            }
        }
    }

    async fn build_output(
        state: &mut BusState,
        output: &OutputConfig,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let input_stream = state
            .input_streams
            .iter()
            .find(|s| match output.av_type {
                OutputAvType::Video => s.is_video(),
                OutputAvType::Audio => s.is_audio(),
            })
            .ok_or_else(|| anyhow::anyhow!("input has no {:?} stream", output.av_type))?;
        let input_stream_index = input_stream.index();
        let need_decoder = Self::try_decoder(input_stream, output)?;
        let need_encoder = Self::try_encoder(input_stream, output)?;
        let is_file_net = matches!(
            &output.dest,
            OutputDest::File { .. } | OutputDest::Net { .. }
        );
        // File/Net decide copy vs transcode per stream and start their
        // decoder/encoder tasks inside the muxer builder; every other
        // dest starts the primary stream's tasks here.
        let mut encoder_key = None;
        if !is_file_net {
            if need_decoder {
                Self::start_decoder_task(state, input_stream_index).await?;
            }
            if need_encoder {
                // A Mux output without an encode config gets the encoder its
                // format implies (e.g. "opus" → libopus), not the generic default.
                let encode = match (&output.encode, &output.dest) {
                    (None, OutputDest::Mux { format }) => mux_format_encode(format),
                    (encode, _) => encode.clone(),
                };
                // Live/streaming outputs keep the lossy (low-latency) path.
                encoder_key = Some(
                    Self::start_encoder_task(state, input_stream_index, encode.as_ref(), false)
                        .await?,
                );
            }
        }

        match &output.dest {
            OutputDest::Raw => {
                Self::create_decoder_raw_output_stream(state, input_stream_index).await
            }
            OutputDest::File { path } => {
                Self::create_mux_to_file(state, path, input_stream_index, output).await
            }
            OutputDest::Net { url, format } => {
                Self::create_mux_to_net(state, url, format.as_deref(), input_stream_index, output)
                    .await
            }
            OutputDest::Mux { format } => match &encoder_key {
                Some(key) => Self::create_mux_output_stream_from_encoder(state, format, key).await,
                None => Self::create_mux_output_stream(state, format, input_stream_index).await,
            },
            OutputDest::Encoded => {
                Self::create_encoded_output_stream(state, input_stream_index, encoder_key.as_ref())
                    .await
            }
            OutputDest::Demuxed => {
                Self::create_demuxed_output_stream(state, input_stream_index).await
            }
        }
    }

    /// The input is still producing (or about to): restarting a stopped
    /// decoder/encoder makes sense. Once it has ended, a fresh task would only
    /// wait forever, so subscribers get end-of-stream instead.
    fn input_running(state: &BusState) -> bool {
        state.input_task.as_ref().is_some_and(|t| t.is_running())
    }

    /// Subscribe to the stream's shared decoder, (re)starting it if it stopped
    /// because nobody was using it.
    async fn subscribe_decoder(
        state: &mut BusState,
        stream_index: usize,
        lossless: bool,
    ) -> anyhow::Result<crate::frame::RawFrameReceiver> {
        for _ in 0..2 {
            Self::start_decoder_task(state, stream_index).await?;
            let task = state
                .decoder_tasks
                .get(&stream_index)
                .ok_or(anyhow::anyhow!("decoder task not found"))?;
            if let Some(rx) = task.try_subscribe(lossless) {
                return Ok(rx);
            }
            if !Self::input_running(state) {
                return Ok(crate::decoder::closed_receiver());
            }
            // It stopped between the check and the subscribe: start afresh.
            state.decoder_tasks.remove(&stream_index);
        }
        Err(anyhow::anyhow!(
            "decoder for stream {stream_index} keeps stopping"
        ))
    }

    /// Subscribe to a shared encoder, (re)starting it if it stopped because
    /// nobody was reading it.
    async fn subscribe_encoder(
        state: &mut BusState,
        key: &EncoderKey,
    ) -> anyhow::Result<RawPacketReceiver> {
        for _ in 0..2 {
            Self::start_encoder_task(state, key.stream_index, key.encode.as_ref(), key.lossless)
                .await?;
            let task = state
                .encoder_tasks
                .get(key)
                .ok_or(anyhow::anyhow!("encoder task not found"))?;
            if let Some(rx) = task.try_subscribe() {
                return Ok(rx);
            }
            if !Self::input_running(state) {
                return Ok(tokio::sync::broadcast::channel(1).1);
            }
            state.encoder_tasks.remove(key);
            state.encoder_output_streams.remove(key);
        }
        Err(anyhow::anyhow!(
            "encoder for stream {} keeps stopping",
            key.stream_index
        ))
    }

    fn try_decoder(input_stream: &AvStream, output: &OutputConfig) -> anyhow::Result<bool> {
        let input_codec = input_stream.parameters().id();

        // RAWVIDEO: packets are raw pixels, no decoder. WRAPPED_AVFRAME: packets wrap AVFrame, need decoder to unwrap.
        if input_codec == ffmpeg_next::codec::Id::RAWVIDEO {
            return Ok(false);
        }

        match &output.dest {
            OutputDest::Raw => Ok(true),
            OutputDest::File { .. } => Ok(false),
            // Mux: need decoder only when encoder is also needed (e.g. WRAPPED_AVFRAME needs unwrap → encode).
            // If input is already the target codec (e.g. H.264 → h264 mux), no decoder needed.
            // For audio passthrough (e.g. AAC → adts mux), no decoder needed.
            OutputDest::Mux { .. } => {
                if input_stream.is_video() {
                    Ok(input_codec == ffmpeg_next::codec::Id::WRAPPED_AVFRAME
                        || Self::try_encoder(input_stream, output).unwrap_or(false))
                } else {
                    // Audio: only decode if encoder is needed (transcode case)
                    Ok(Self::try_encoder(input_stream, output).unwrap_or(false))
                }
            }
            OutputDest::Net { .. } => {
                // Audio passthrough to net doesn't need decoder
                if input_stream.is_audio() {
                    Ok(Self::try_encoder(input_stream, output).unwrap_or(false))
                } else {
                    Ok(true)
                }
            }
            OutputDest::Encoded => Ok(true),
            // Pure passthrough: no decoder, no encoder.
            OutputDest::Demuxed => Ok(false),
        }
    }

    fn try_encoder(input_stream: &AvStream, output: &OutputConfig) -> anyhow::Result<bool> {
        let input_codec = input_stream.parameters().id();

        if let OutputDest::Raw = output.dest {
            return Ok(false);
        }
        if let OutputDest::Demuxed = output.dest {
            return Ok(false);
        }

        // Video-specific raw codecs
        if input_stream.is_video()
            && (input_codec == ffmpeg_next::codec::Id::RAWVIDEO
                || input_codec == ffmpeg_next::codec::Id::WRAPPED_AVFRAME)
        {
            return Ok(true);
        }

        if let OutputDest::Encoded = output.dest {
            return Ok(true);
        }

        // Mux format requires encoded packets; use encoder when input is not already that codec
        if let OutputDest::Mux { format } = &output.dest {
            let need_encode = match format.as_str() {
                "h264" => input_codec != ffmpeg_next::codec::Id::H264,
                "hevc" | "h265" => input_codec != ffmpeg_next::codec::Id::HEVC,
                "aac" | "adts" => input_codec != ffmpeg_next::codec::Id::AAC,
                "opus" => input_codec != ffmpeg_next::codec::Id::OPUS,
                _ => false,
            };
            if need_encode {
                return Ok(true);
            }
        }

        // Adaptive: an explicit encode config forces a transcode only when the
        // requested params actually differ from the input; if they match, the
        // stream is copied through unchanged (no decoder, no encoder).
        if let Some(encode) = &output.encode {
            return Ok(Self::encode_needed(input_stream, encode));
        }

        Ok(false)
    }

    /// Map an [`EncodeConfig::codec`] name to its codec id (best-effort). Covers
    /// the codecs this pipeline emits; unknown names yield `None` (treated as a
    /// codec change, i.e. transcode).
    fn codec_id_from_name(name: &str) -> Option<ffmpeg_next::codec::Id> {
        use ffmpeg_next::codec::Id;
        Some(match name.to_ascii_lowercase().as_str() {
            "h264" | "avc" | "libx264" => Id::H264,
            "hevc" | "h265" | "libx265" => Id::HEVC,
            "aac" => Id::AAC,
            "opus" | "libopus" => Id::OPUS,
            "mjpeg" => Id::MJPEG,
            "vp8" | "libvpx" => Id::VP8,
            "vp9" | "libvpx-vp9" => Id::VP9,
            "av1" => Id::AV1,
            "mp3" | "libmp3lame" => Id::MP3,
            "rawvideo" => Id::RAWVIDEO,
            _ => return None,
        })
    }

    /// Whether an explicit encode config actually requires a transcode of the
    /// input stream, or whether it can be copied through unchanged. Only the
    /// *structural* parameters a stream-copy cannot alter are compared: the
    /// codec, plus geometry (video) or sample rate + channel count (audio).
    /// Quality knobs (bitrate, preset, pixel_format) do not by themselves force
    /// a transcode when the structural params already match.
    fn encode_needed(input_stream: &AvStream, encode: &EncodeConfig) -> bool {
        Self::encode_needed_params(
            input_stream.parameters().id(),
            input_stream.is_video(),
            input_stream.width(),
            input_stream.height(),
            input_stream.sample_rate(),
            input_stream.channels(),
            encode,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_needed_params(
        input_codec: ffmpeg_next::codec::Id,
        is_video: bool,
        width: u32,
        height: u32,
        sample_rate: u32,
        channels: u32,
        encode: &EncodeConfig,
    ) -> bool {
        // A different (or unrecognized) target codec always requires a transcode.
        match Self::codec_id_from_name(&encode.codec) {
            Some(target) if target == input_codec => {}
            _ => return true,
        }
        if is_video {
            encode.width.is_some_and(|w| w != width) || encode.height.is_some_and(|h| h != height)
        } else {
            encode.sample_rate.is_some_and(|sr| sr != sample_rate)
                || encode.channels.is_some_and(|c| c != channels)
        }
    }

    /// Mux to a real file path (seekable). Standard MP4 any player can open.
    /// Per stream, copies the demuxed input or muxes the transcoded encoder
    /// output; `output.include_audio` also carries the audio stream.
    async fn create_mux_to_file(
        state: &mut BusState,
        path: &str,
        primary_index: usize,
        output: &OutputConfig,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let mut plan = Self::build_mux_plan(state, primary_index, output)?;
        Self::start_mux_transcoders(state, &mut plan).await?;
        Self::spawn_multi_stream_mux(state, MuxTarget::File(path.to_string()), plan).await
    }

    /// Mux to a network URL (rtmp://, rtsp://, ...). Per stream, copies the
    /// demuxed input or muxes the transcoded encoder output.
    async fn create_mux_to_net(
        state: &mut BusState,
        url: &str,
        format: Option<&str>,
        primary_index: usize,
        output: &OutputConfig,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let mut plan = Self::build_mux_plan(state, primary_index, output)?;
        Self::start_mux_transcoders(state, &mut plan).await?;
        Self::spawn_multi_stream_mux(
            state,
            MuxTarget::Net {
                url: url.to_string(),
                format: format.map(str::to_string),
            },
            plan,
        )
        .await
    }

    /// Plan the streams a File/Net output muxes and whether each is copied or
    /// transcoded. The primary (`av_type`) stream uses `output.encode`; the
    /// audio stream carried via `include_audio` uses `output.audio_encode`.
    /// A stream is transcoded when its encode config differs from the input
    /// params (see [`Self::encode_needed`]); otherwise it is copied.
    fn build_mux_plan(
        state: &BusState,
        primary_index: usize,
        output: &OutputConfig,
    ) -> anyhow::Result<Vec<MuxPlanEntry>> {
        let primary = state
            .input_streams
            .iter()
            .find(|s| s.index() == primary_index)
            .ok_or(anyhow::anyhow!("no matching stream in input"))?;
        let mut plan = vec![Self::plan_entry(primary, output.encode.as_ref())];

        if output.include_audio
            && primary.is_video()
            && let Some(audio) = state.input_streams.iter().find(|s| s.is_audio())
        {
            plan.push(Self::plan_entry(audio, output.audio_encode.as_ref()));
        }
        Ok(plan)
    }

    fn plan_entry(stream: &AvStream, encode: Option<&EncodeConfig>) -> MuxPlanEntry {
        let input_codec = stream.parameters().id();
        let transcode = encode.is_some_and(|e| Self::encode_needed(stream, e));

        let codec_id = if transcode {
            encode
                .and_then(|e| Self::codec_id_from_name(&e.codec))
                .unwrap_or(input_codec)
        } else {
            input_codec
        };
        MuxPlanEntry {
            input_index: stream.index(),
            transcode,
            encode: if transcode { encode.cloned() } else { None },
            codec_id,
            encoder_key: None,
        }
    }

    /// Start a decoder + encoder task for each transcoded stream in the plan.
    async fn start_mux_transcoders(
        state: &mut BusState,
        plan: &mut [MuxPlanEntry],
    ) -> anyhow::Result<()> {
        // Non-live sources transcode losslessly: a file decoded in a burst must
        // not drop frames (gaps, A/V drift). Live sources must never be slowed
        // by a slow encoder, so they drop frames instead.
        let lossless = !state.live;
        for entry in plan.iter_mut().filter(|e| e.transcode) {
            Self::start_decoder_task(state, entry.input_index).await?;
            entry.encoder_key = Some(
                Self::start_encoder_task(state, entry.input_index, entry.encode.as_ref(), lossless)
                    .await?,
            );
        }
        Ok(())
    }

    /// Build the muxer and spawn the task that merges every planned stream —
    /// copied input packets plus transcoded encoder packets — into one
    /// container. Each output track keeps its input stream index so the muxer's
    /// index-keyed mapping stays unambiguous.
    async fn spawn_multi_stream_mux(
        state: &mut BusState,
        target: MuxTarget,
        plan: Vec<MuxPlanEntry>,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let label = match &target {
            MuxTarget::File(path) => path.clone(),
            MuxTarget::Net { url, .. } => url.clone(),
        };

        // One output stream per planned stream; collect the packet sources.
        let mut out_streams: Vec<AvStream> = Vec::new();
        let mut copied_indices: HashSet<usize> = HashSet::new();
        let mut enc_receivers: Vec<(usize, RawPacketReceiver)> = Vec::new();
        let mut primary_av: Option<AvStream> = None;

        for entry in &plan {
            let input_stream = state
                .input_streams
                .iter()
                .find(|s| s.index() == entry.input_index)
                .ok_or(anyhow::anyhow!("no matching stream in input"))?
                .clone();
            let out_stream = match &entry.encoder_key {
                // Use the encoder's real output params (rate/channels/dims +
                // extradata), captured when its task started, so the muxed
                // header matches the transcoded packets.
                Some(key) => state
                    .encoder_output_streams
                    .get(key)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "no encoder output stream for transcoded input {}",
                            entry.input_index
                        )
                    })?,
                None => input_stream,
            };
            out_streams.push(out_stream.clone());
            if primary_av.is_none() {
                primary_av = Some(out_stream.clone());
            }
            match &entry.encoder_key {
                Some(key) => {
                    let recv = Self::subscribe_encoder(state, key).await?;
                    enc_receivers.push((entry.input_index, recv));
                }
                None => {
                    copied_indices.insert(entry.input_index);
                }
            }
        }
        let primary_av = primary_av.ok_or(anyhow::anyhow!("mux plan is empty"))?;
        // Open and write the header now so a dead target (e.g. nothing
        // listening on an RTSP URL) fails this add_output call. Both can block
        // (file create, network connect): keep them off the async workers.
        let is_net = matches!(target, MuxTarget::Net { .. });
        let shutdown = state.shutdown.clone();
        let output =
            tokio::task::spawn_blocking(move || Self::open_mux(target, &out_streams, shutdown))
                .await
                .map_err(|e| anyhow::anyhow!("open mux task failed: {e}"))?
                .map_err(|e| anyhow::anyhow!("mux open({}): {:#}", label, e))?;

        let input_task = state
            .input_task
            .as_ref()
            .ok_or(anyhow::anyhow!("input task not found"))?;
        // For a non-live source File/Net outputs are lossless end to end: the
        // copied packets come straight from the input, so the reader must not
        // outrun this muxer. A live source is never held back.
        let live = state.live;
        if !live {
            input_task.set_lossless();
        }
        // Subscribe to the input only when something is copied from it. A
        // fully transcoded mux ends on its encoders' EOFs; waiting on an input
        // EOF too would hang forever when this output joins after a fast
        // source (e.g. a short file) has already been read to the end.
        let input_receiver = (!copied_indices.is_empty()).then(|| input_task.subscribe());

        tokio::spawn(async move {
            // One MuxSignal stream per source. A source's channel may stay open
            // after its logical end (the input/encoder tasks keep a sender), so
            // termination is driven by the EOF *signal* (one per source), not by
            // channel close.
            let mut sources: Vec<Pin<Box<dyn Stream<Item = MuxSignal> + Send>>> = Vec::new();
            let copied = Arc::new(copied_indices);
            if let Some(input_receiver) = input_receiver {
                let copied = copied.clone();
                let s = BroadcastStream::new(input_receiver).filter_map(move |r| {
                    let copied = copied.clone();
                    async move {
                        match r {
                            Ok(RawPacketCmd::Data(p)) if copied.contains(&p.index()) => {
                                Some(MuxSignal::Packet(p.index(), p))
                            }
                            Ok(RawPacketCmd::Data(_)) => None, // packet for a transcoded stream
                            Ok(RawPacketCmd::EOF) => Some(MuxSignal::Eof),
                            // Lagged: copied packets were lost.
                            Err(_) => Some(MuxSignal::Gap(copied.iter().copied().collect())),
                        }
                    }
                });
                sources.push(Box::pin(s));
            }
            for (idx, recv) in enc_receivers {
                let s = BroadcastStream::new(recv).filter_map(move |r| async move {
                    match r {
                        Ok(RawPacketCmd::Data(p)) => Some(MuxSignal::Packet(idx, p)),
                        Ok(RawPacketCmd::EOF) => Some(MuxSignal::Eof),
                        // Lagged: encoded packets were lost.
                        Err(_) => Some(MuxSignal::Gap(vec![idx])),
                    }
                });
                sources.push(Box::pin(s));
            }

            // Disk / network writes block: a dedicated thread does them, fed
            // through a bounded queue so a slow sink backpressures the merge.
            let (write_tx, mut write_rx) =
                tokio::sync::mpsc::channel::<(usize, RawPacket)>(MUX_WRITE_QUEUE);
            let writer = tokio::task::spawn_blocking(move || {
                let mut output = output;
                while let Some((idx, packet)) = write_rx.blocking_recv() {
                    if let Err(e) = output.write_packet(idx, packet) {
                        if is_net && is_connection_error(&e) {
                            log::error!("mux network output lost: {e:#}; stopping it");
                            break;
                        }
                        log::error!("mux write_packet error: {:#?}", e);
                    }
                }
                if let Err(e) = output.finish() {
                    log::error!(
                        "mux finish error: {:#?}\nbacktrace:\n{}",
                        e,
                        Backtrace::capture()
                    );
                }
            });

            let total_sources = sources.len();
            let mut eofs = 0usize;
            let mut gate = KeyframeGate::default();
            let mut merged = futures::stream::select_all(sources);
            while let Some(sig) = merged.next().await {
                match sig {
                    MuxSignal::Packet(idx, packet) => {
                        if !gate.admit(idx, packet.is_key()) {
                            continue;
                        }
                        if live {
                            // Never wait on a slow sink: drop and resync.
                            match write_tx.try_send((idx, packet)) {
                                Ok(()) => {}
                                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                                    if gate.note_drop(idx) {
                                        log::warn!(
                                            "mux {}: sink too slow, dropped {} packets (live)",
                                            label,
                                            gate.dropped
                                        );
                                    }
                                }
                                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => break,
                            }
                        } else if write_tx.send((idx, packet)).await.is_err() {
                            break; // writer gone
                        }
                    }
                    MuxSignal::Gap(indices) => {
                        for idx in indices {
                            gate.mark_gap(idx);
                        }
                    }
                    MuxSignal::Eof => {
                        eofs += 1;
                        if eofs >= total_sources {
                            break;
                        }
                    }
                }
            }
            // Closing the queue lets the writer drain, write the trailer, exit.
            drop(write_tx);
            let _ = writer.await;
            log::info!("mux finished: {}", label);
        });

        Ok((
            primary_av,
            Box::pin(futures::stream::empty::<Option<VideoFrame>>()),
        ))
    }

    /// Create the container for `target`, add `streams`, and write the header
    /// (which for RTSP is where the connection is made). Blocking: call it
    /// from a blocking thread.
    fn open_mux(
        target: MuxTarget,
        streams: &[AvStream],
        shutdown: Arc<std::sync::atomic::AtomicBool>,
    ) -> anyhow::Result<AvOutput> {
        let mut output = match target {
            // Not interruptible: aborting a local file mid-trailer would leave
            // a corrupt recording, and disk writes do not hang like sockets.
            MuxTarget::File(path) => AvOutput::new(&path, None, None)?,
            // Network: interruptible by `Bus::stop`, with an I/O timeout so a
            // peer that stops reading fails the output instead of hanging it.
            MuxTarget::Net { url, format } => {
                let options = net_output_options(format.as_deref());
                AvOutput::open_network(&url, format.as_deref(), Some(options), shutdown)?
            }
        };
        for stream in streams {
            output.add_stream(stream)?;
        }
        output.write_header()?;
        Ok(output)
    }

    async fn create_encoded_output_stream(
        state: &mut BusState,
        input_stream_index: usize,
        encoder_key: Option<&EncoderKey>,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let av = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("stream not found"))?
            .clone();
        let key = encoder_key.ok_or(anyhow::anyhow!("encoder task not found"))?;
        let encoder_receiver = Self::subscribe_encoder(state, key).await?;

        let stream = BroadcastStream::new(encoder_receiver).filter_map(|r| async move {
            match r {
                Ok(RawPacketCmd::Data(packet)) => Some(Some(VideoFrame::from(packet))),
                Ok(RawPacketCmd::EOF) => Some(None),
                Err(_) => None,
            }
        });

        Ok((av.clone(), Box::pin(stream)))
    }

    /// Mux encoded packets (from encoder_tasks) into format (e.g. "h264"). Used when input
    /// was not already that codec and encoder was started.
    async fn create_mux_output_stream_from_encoder(
        state: &mut BusState,
        format: &str,
        encoder_key: &EncoderKey,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let codec_id = mux_format_codec(format).ok_or_else(|| {
            anyhow::anyhow!("unsupported mux format for encoder output: {format}")
        })?;
        // The encoder's real output params (incl. extradata), not the input's.
        let encoder_output_stream = state
            .encoder_output_streams
            .get(encoder_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no encoder output stream"))?;
        let encoded = encoder_output_stream.parameters().id();
        if encoded != codec_id {
            anyhow::bail!("mux format {format} needs {codec_id:?}, encoder makes {encoded:?}");
        }
        let encoder_receiver = Self::subscribe_encoder(state, encoder_key).await?;

        let mut stream = AvOutputStream::new(format)?;
        stream.add_stream(&encoder_output_stream)?;
        let (writer, reader) = stream.into_split();
        let index = encoder_output_stream.index();
        tokio::task::spawn_blocking(move || {
            run_mux_stream_writer(writer, encoder_receiver, index, true)
        });

        Ok((
            encoder_output_stream,
            Box::pin(reader.map(|pkg| Some(VideoFrame::from(pkg)))),
        ))
    }

    async fn create_mux_output_stream(
        state: &mut BusState,
        format: &str,
        input_stream_index: usize,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let input_receiver = state
            .input_task
            .as_ref()
            .ok_or(anyhow::anyhow!("input task not found"))?
            .subscribe();

        let target_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("no matching stream in input"))?
            .clone();
        let mut stream = AvOutputStream::new(format)?;
        stream.add_stream(&target_stream)?;
        let (writer, reader) = stream.into_split();
        let index = target_stream.index();
        tokio::task::spawn_blocking(move || {
            run_mux_stream_writer(writer, input_receiver, index, false)
        });

        Ok((
            target_stream,
            Box::pin(reader.map(|pkg| Some(VideoFrame::from(pkg)))),
        ))
    }

    /// Subscribe to demuxed input packets and emit each packet for the
    /// requested stream as a `VideoFrame`. No decoder, no encoder, no muxer —
    /// the packet bytes are exactly what came out of the input demuxer
    /// (raw codec frames, no container framing). Suitable for codec-aware
    /// downstream consumers like ZLMediaKit.
    async fn create_demuxed_output_stream(
        state: &mut BusState,
        input_stream_index: usize,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let mut input_receiver = state
            .input_task
            .as_ref()
            .ok_or(anyhow::anyhow!("input task not found"))?
            .subscribe();

        let target_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("no matching stream in input"))?
            .clone();
        let target_stream_index = target_stream.index();

        let (tx, rx) = tokio::sync::mpsc::channel::<Option<VideoFrame>>(256);
        tokio::spawn(async move {
            loop {
                match input_receiver.recv().await {
                    Ok(RawPacketCmd::Data(packet)) => {
                        if packet.index() == target_stream_index {
                            if tx.send(Some(VideoFrame::from(packet))).await.is_err() {
                                break;
                            }
                        }
                    }
                    Ok(RawPacketCmd::EOF) => {
                        let _ = tx.send(None).await;
                        break;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("demuxed input_receiver lagged, dropped {} messages", n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            log::info!("demuxed stream finished");
        });

        Ok((
            target_stream,
            Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)),
        ))
    }

    async fn create_decoder_raw_output_stream(
        state: &mut BusState,
        stream_index: usize,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let av = state
            .input_streams
            .iter()
            .find(|s| s.index() == stream_index)
            .ok_or(anyhow::anyhow!("stream not found"))?;
        let av = av.clone();
        let stream =
            BroadcastStream::new(Self::subscribe_decoder(state, stream_index, false).await?)
                .filter_map(|cmd| async move {
                    match cmd {
                        Ok(RawFrameCmd::Data(frame)) => match VideoFrame::try_from(frame) {
                            Ok(frame) => Some(Some(frame)),
                            Err(e) => {
                                log::debug!("raw output: skip unconvertible frame: {:#}", e);
                                None
                            }
                        },
                        Ok(RawFrameCmd::EOF) => Some(None),
                        // A lossy subscriber that falls behind just loses its oldest
                        // frames; that is not end of stream.
                        Err(e) => {
                            log::debug!("raw output: {}", e);
                            None
                        }
                    }
                });

        Ok((av.clone(), Box::pin(stream)))
    }

    /// Ensure the input + audio decoder are running and return a subscription to
    /// the decoded-audio broadcast. Mirrors the `OutputDest::Raw` audio path.
    async fn subscribe_audio_internal(
        state: &mut BusState,
    ) -> anyhow::Result<crate::frame::RawFrameReceiver> {
        if state.input_task.is_none() && state.input_config.is_some() {
            Self::prepare_input_task(state).await?;
        }
        let audio_index = state
            .input_streams
            .iter()
            .find(|s| s.is_audio())
            .ok_or_else(|| anyhow::anyhow!("pipe has no audio stream"))?
            .index();
        let receiver = Self::subscribe_decoder(state, audio_index, false).await?;
        Self::auto_start_input(state).await?;
        Ok(receiver)
    }

    /// Ensure the input + video decoder are running and return a subscription to
    /// the decoded-video broadcast. Mirrors `subscribe_audio_internal`.
    async fn subscribe_video_internal(
        state: &mut BusState,
    ) -> anyhow::Result<crate::frame::RawFrameReceiver> {
        if state.input_task.is_none() && state.input_config.is_some() {
            Self::prepare_input_task(state).await?;
        }
        let video_index = state
            .input_streams
            .iter()
            .find(|s| s.is_video())
            .ok_or_else(|| anyhow::anyhow!("pipe has no video stream"))?
            .index();
        let receiver = Self::subscribe_decoder(state, video_index, false).await?;
        Self::auto_start_input(state).await?;
        Ok(receiver)
    }

    async fn add_input_internal(
        state: &mut BusState,
        input: InputConfig,
        options: Option<HashMap<String, String>>,
        live: Option<bool>,
    ) -> anyhow::Result<()> {
        if state.input_config.is_some() {
            return Err(anyhow::anyhow!("input already exists"));
        } else {
            state.live = live.unwrap_or_else(|| infer_live(&input));
            state.input_config = Some(input);
            state.input_options = options;
        }

        if !state.output_config.is_empty() && state.input_task.is_none() {
            Self::prepare_input_task(state).await?;
            Self::auto_start_input(state).await?;
        }
        Ok(())
    }

    /// Reads (width, height, pixel_format) from video codec parameters (for raw video).
    fn raw_video_params_from_parameters(
        params: &ffmpeg_next::codec::Parameters,
    ) -> (u32, u32, ffmpeg_next::format::Pixel) {
        unsafe {
            let ptr = params.as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
            let w = (*ptr).width.max(0) as u32;
            let h = (*ptr).height.max(0) as u32;
            let fmt = (*ptr).format;
            let pixel_format = ffmpeg_next::format::Pixel::from(std::mem::transmute::<
                i32,
                ffmpeg_next::ffi::AVPixelFormat,
            >(fmt));
            (w, h, pixel_format)
        }
    }

    /// Fallback when codec parameters report 0x0 (e.g. WRAPPED_AVFRAME before first frame).
    fn ensure_video_dimensions(width: u32, height: u32) -> (u32, u32) {
        const FALLBACK_W: u32 = 320;
        const FALLBACK_H: u32 = 240;
        let w = if width == 0 { FALLBACK_W } else { width };
        let h = if height == 0 { FALLBACK_H } else { height };
        (w, h)
    }

    /// Build encoder options from EncodeConfig for faster encoding (preset, bitrate).
    fn encoder_options_from_config(encode: Option<&EncodeConfig>) -> Option<Dictionary<'_>> {
        let encode = encode?;
        let mut opts = Dictionary::new();
        opts.set("preset", encode.preset.as_deref().unwrap_or("ultrafast"));
        opts.set("tune", "zerolatency");
        if let Some(b) = encode.bitrate {
            opts.set("b", b.to_string().as_str());
        }
        Some(opts)
    }

    /// Open a video encoder on a blocking thread (probing hardware candidates
    /// can be slow). Options are built inside: `Dictionary` is not `Send`.
    async fn open_video_encoder(
        stream: &AvStream,
        settings: Settings,
        encode: Option<&EncodeConfig>,
    ) -> anyhow::Result<Encoder> {
        let stream = stream.clone();
        let encode = encode.cloned();
        open_blocking(move || {
            let options = Self::encoder_options_from_config(encode.as_ref());
            Encoder::new(&stream, settings, options)
        })
        .await
    }

    fn encoder_codec_from_config(encode: Option<&EncodeConfig>) -> String {
        encode
            .map(|e| e.codec.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("h264")
            .to_string()
    }

    /// Build audio encoder settings from EncodeConfig.
    fn audio_settings_from_config(encode: Option<&EncodeConfig>) -> AudioSettings {
        match encode {
            Some(cfg) => AudioSettings {
                codec: Some(cfg.codec.clone()),
                sample_rate: cfg.sample_rate,
                channels: cfg.channels,
                bitrate: cfg.audio_bitrate,
                ..AudioSettings::default()
            },
            None => AudioSettings::default(),
        }
    }

    /// Start (or reuse) the encoder for `(stream, encode, lossless)` and return
    /// its key. Same key → shared encoder; a different config gets its own.
    async fn start_encoder_task(
        state: &mut BusState,
        input_stream_index: usize,
        encode: Option<&EncodeConfig>,
        lossless: bool,
    ) -> anyhow::Result<EncoderKey> {
        let key = EncoderKey {
            stream_index: input_stream_index,
            encode: encode.cloned(),
            lossless,
        };
        // Reuse a running encoder; keep an ended one when the input is done
        // too (subscribers then get end-of-stream, see `subscribe_encoder`).
        if let Some(task) = state.encoder_tasks.get(&key)
            && (task.is_running() || !Self::input_running(state))
        {
            return Ok(key);
        }
        // Owned copy: the decoder subscription below needs `state` mutably.
        let input_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("stream not found"))?
            .clone();
        let input_stream = &input_stream;

        // Audio encoder path
        if input_stream.is_audio() {
            let encoder_task = EncoderTask::new_auto_stop();
            let encoder_receiver =
                Self::subscribe_decoder(state, input_stream_index, lossless).await?;
            let audio_settings = Self::audio_settings_from_config(encode);
            let stream = input_stream.clone();
            let encoder =
                open_blocking(move || Encoder::new_audio(&stream, audio_settings, None)).await?;
            let out_stream = encoder.output_stream(input_stream_index);
            encoder_task
                .start(encoder, encoder_receiver, lossless)
                .await;
            state.encoder_tasks.insert(key.clone(), encoder_task);
            state.encoder_output_streams.insert(key.clone(), out_stream);
            return Ok(key);
        }

        // Video encoder path
        let codec_id = input_stream.parameters().id();
        let encoder_task = EncoderTask::new_auto_stop();
        // Encoder-derived output stream descriptor for the muxer, set in each branch.
        let out_stream: AvStream;
        // Only RAWVIDEO has raw pixel data in packets; use packet->frame conversion.
        // WRAPPED_AVFRAME packets wrap AVFrame (not raw pixels), so use decoder path.
        if codec_id == ffmpeg_next::codec::Id::RAWVIDEO {
            let (width, height, pixel_format) =
                Self::raw_video_params_from_parameters(input_stream.parameters());
            let (width, height) = Self::ensure_video_dimensions(width, height);
            let codec = Self::encoder_codec_from_config(encode);
            let encoder_settings = Settings {
                width,
                height,
                pixel_format: pixel_format_for_libx264(pixel_format),
                codec: Some(codec),
                ..Settings::default()
            };
            let packet_receiver: tokio::sync::broadcast::Receiver<RawPacketCmd> = state
                .input_task
                .as_ref()
                .ok_or(anyhow::anyhow!("input task not found"))?
                .subscribe();
            /// Raw frames; balance memory vs avoiding Lagged (dropped frames break stream).
            const RAW_FRAME_CHAN_CAP: usize = 16;
            let (frame_tx, frame_rx) =
                tokio::sync::broadcast::channel::<RawFrameCmd>(RAW_FRAME_CHAN_CAP);
            let encoder = Self::open_video_encoder(input_stream, encoder_settings, encode).await?;
            // Spawn task: packet -> frame conversion, then forward to encoder
            {
                let mut packet_rx = packet_receiver;
                let frame_tx = frame_tx;
                tokio::spawn(async move {
                    loop {
                        let msg = match packet_rx.recv().await {
                            Ok(RawPacketCmd::Data(packet)) => {
                                match packet_to_raw_video_frame(packet, width, height, pixel_format)
                                {
                                    Ok(frame) => RawFrameCmd::Data(frame),
                                    Err(_) => continue,
                                }
                            }
                            Ok(RawPacketCmd::EOF) => RawFrameCmd::EOF,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                log::warn!("rawvideo relay lagged, lost {} packets", n);
                                continue;
                            }
                            // Input gone: stop (dropping frame_tx ends the encoder relay).
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        };
                        let is_eof = matches!(msg, RawFrameCmd::EOF);
                        if lossless {
                            while frame_tx.len() >= RAW_FRAME_CHAN_CAP
                                && frame_tx.receiver_count() > 0
                            {
                                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                            }
                        }
                        // No receiver left: the encoder is gone, stop relaying.
                        if frame_tx.send(msg).is_err() || is_eof {
                            break;
                        }
                    }
                });
            }
            out_stream = encoder.output_stream(input_stream_index);
            encoder_task.start(encoder, frame_rx, lossless).await;
        } else {
            let encoder_receiver =
                Self::subscribe_decoder(state, input_stream_index, lossless).await?;
            // Decoded path: decoder outputs RawFrame; encoder needs correct size/format.
            // For WRAPPED_AVFRAME (e.g. lavfi testsrc), use stream params so output resolution matches source.
            let codec = Self::encoder_codec_from_config(encode);
            let encoder_settings = if codec_id == ffmpeg_next::codec::Id::WRAPPED_AVFRAME {
                let (width, height, pixel_format) =
                    Self::raw_video_params_from_parameters(input_stream.parameters());
                let (width, height) = Self::ensure_video_dimensions(width, height);
                Settings {
                    width,
                    height,
                    pixel_format: pixel_format_for_libx264(pixel_format),
                    codec: Some(codec.clone()),
                    ..Settings::default()
                }
            } else {
                // Decoded video transcode: size the encoder to the input (so a
                // codec-only transcode preserves resolution), honoring explicit
                // width/height overrides. The encoder's send_frame scaler handles
                // any resize/format conversion.
                let target_w = encode
                    .and_then(|e| e.width)
                    .unwrap_or_else(|| input_stream.width());
                let target_h = encode
                    .and_then(|e| e.height)
                    .unwrap_or_else(|| input_stream.height());
                let (target_w, target_h) = Self::ensure_video_dimensions(target_w, target_h);
                Settings {
                    width: target_w,
                    height: target_h,
                    pixel_format: ffmpeg_next::format::Pixel::YUV420P,
                    codec: Some(codec),
                    ..Settings::default()
                }
            };
            let encoder = Self::open_video_encoder(input_stream, encoder_settings, encode).await?;
            out_stream = encoder.output_stream(input_stream_index);
            encoder_task
                .start(encoder, encoder_receiver, lossless)
                .await;
        }

        state.encoder_tasks.insert(key.clone(), encoder_task);
        state.encoder_output_streams.insert(key.clone(), out_stream);
        Ok(key)
    }

    /// Start the stream's shared decoder if not running. One decoder per input
    /// stream; each subscriber picks its own loss policy (see `DecoderTask::subscribe`).
    async fn start_decoder_task(
        state: &mut BusState,
        input_stream_index: usize,
    ) -> anyhow::Result<()> {
        // Reuse a running decoder; keep an ended one when the input is done
        // too (subscribers then get end-of-stream, see `subscribe_decoder`).
        if let Some(task) = state.decoder_tasks.get(&input_stream_index)
            && (task.is_running() || !Self::input_running(state))
        {
            return Ok(());
        }
        let input_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("stream not found"))?;
        let codec_id = input_stream.parameters().id();
        if codec_id == ffmpeg_next::codec::Id::RAWVIDEO {
            return Ok(());
        }
        let decoder_receiver = state
            .input_task
            .as_ref()
            .ok_or(anyhow::anyhow!("input task not found"))?
            .subscribe();
        let stream = input_stream.clone();
        // Probing hardware decoder candidates can be slow: off the async workers.
        let decoder = open_blocking(move || Decoder::new(&stream)).await?;
        let decoder_task = DecoderTask::new_auto_stop();
        decoder_task.start(decoder, decoder_receiver).await;
        state.decoder_tasks.insert(input_stream_index, decoder_task);

        Ok(())
    }

    async fn prepare_input_task(state: &mut BusState) -> anyhow::Result<()> {
        if state.input_task.is_some() {
            return Ok(());
        }
        let (url, format) = match state.input_config.as_ref() {
            Some(InputConfig::Net { url }) => (url.clone(), None),
            Some(InputConfig::File { path }) => (path.clone(), None),
            Some(InputConfig::Device { display, format }) => {
                (display.clone(), Some(format.clone()))
            }
            None => return Err(anyhow::anyhow!("input config is not set")),
        };
        let options = rtsp_input_options(&url, state.input_options.clone());
        let shutdown = state.shutdown.clone();
        // Opening probes the source (an RTSP connect can take seconds): run it
        // on a blocking thread so the async workers stay free.
        let input = tokio::task::spawn_blocking(move || {
            let options = options.map(|o| {
                ffmpeg_next::Dictionary::from_iter(o.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            });
            // Interruptible: `Bus::stop` aborts an open stuck on a dead peer.
            AvInput::open(&url, format.as_deref(), options, Some(shutdown))
        })
        .await
        .map_err(|e| anyhow::anyhow!("open input task failed: {e}"))??;

        let streams = input.streams();
        log::info!("start add input streams:");
        for (index, stream) in streams {
            log::info!(
                "stream index: {}, stream id: {:#?}, time_base: {:#?}",
                index,
                stream.parameters().id(),
                stream.time_base()
            );
            state.input_streams.push(stream.clone());
        }

        state.input_task = Some(AvInputTask::new());
        state.pending_input = Some(input);
        Ok(())
    }

    /// Start reading the input, unless this bus defers that to [`Bus::start`]
    /// and it has not been called yet.
    async fn auto_start_input(state: &mut BusState) -> anyhow::Result<()> {
        if state.deferred && !state.started {
            return Ok(());
        }
        Self::start_input_task(state).await
    }

    async fn start_internal(state: &mut BusState) -> anyhow::Result<()> {
        if state.input_config.is_none() {
            anyhow::bail!("start: no input");
        }
        state.started = true;
        Self::prepare_input_task(state).await?;
        Self::start_input_task(state).await
    }

    async fn start_input_task(state: &mut BusState) -> anyhow::Result<()> {
        let input = match state.pending_input.take() {
            Some(input) => input,
            None => return Ok(()),
        };

        if let Some(task) = state.input_task.as_ref() {
            task.start(input).await;
        }

        Ok(())
    }

    /// Set the input. Whether it is a live source is inferred (files are not,
    /// network streams and devices are); see [`Bus::add_input_with_live`].
    pub async fn add_input(
        &self,
        input: InputConfig,
        options: Option<HashMap<String, String>>,
    ) -> anyhow::Result<()> {
        self.send_add_input(input, options, None).await
    }

    /// Set the input, saying explicitly whether it is live. A live source is
    /// never slowed down for a lagging File/Net output: that output drops
    /// packets (resuming at the next keyframe) instead. A non-live source is
    /// read only as fast as its slowest File/Net output, losing nothing.
    pub async fn add_input_with_live(
        &self,
        input: InputConfig,
        options: Option<HashMap<String, String>>,
        live: bool,
    ) -> anyhow::Result<()> {
        self.send_add_input(input, options, Some(live)).await
    }

    async fn send_add_input(
        &self,
        input: InputConfig,
        options: Option<HashMap<String, String>>,
        live: Option<bool>,
    ) -> anyhow::Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(BusCommand::AddInput {
                input,
                options,
                live,
                result: tx,
            })
            .await?;
        rx.await?
    }

    /// Start reading the input (see [`Bus::new_deferred`]). Idempotent; on a
    /// non-deferred bus it just starts reading early.
    pub async fn start(&self) -> anyhow::Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(BusCommand::Start { result: tx }).await?;
        rx.await?
    }

    pub async fn remove_input(&self) -> anyhow::Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx.send(BusCommand::RemoveInput { result: tx }).await?;
        rx.await?
    }

    pub async fn add_output(
        &self,
        output: OutputConfig,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(BusCommand::AddOutput { output, result: tx })
            .await?;
        rx.await?
    }

    /// Subscribe to this pipe's decoded-audio broadcast, starting the audio
    /// decoder if needed. The receiver yields `RawFrameCmd` (filter `Audio`).
    pub async fn subscribe_audio(&self) -> anyhow::Result<crate::frame::RawFrameReceiver> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(BusCommand::SubscribeAudio { result: tx })
            .await?;
        rx.await?
    }

    /// Subscribe to this pipe's decoded-video broadcast, starting the video
    /// decoder if needed. The receiver yields `RawFrameCmd` (filter `Video`).
    pub async fn subscribe_video(&self) -> anyhow::Result<crate::frame::RawFrameReceiver> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(BusCommand::SubscribeVideo { result: tx })
            .await?;
        rx.await?
    }

    pub fn stop(&self) {
        self.shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.cancel.cancel();
    }
}

impl Drop for Bus {
    fn drop(&mut self) {
        self.stop();
    }
}

struct BusState {
    input_config: Option<InputConfig>,
    input_options: Option<HashMap<String, String>>,
    output_config: HashMap<String, OutputConfig>,
    input_task: Option<AvInputTask>,
    pending_input: Option<AvInput>,
    input_streams: Vec<AvStream>,
    decoder_tasks: HashMap<usize, DecoderTask>,
    encoder_tasks: HashMap<EncoderKey, EncoderTask>,
    /// Encoder-derived output stream descriptors, keyed like `encoder_tasks`.
    /// Populated when an encoder task starts; the muxer uses these (not the
    /// input params) for transcoded streams so the header matches the packets.
    encoder_output_streams: HashMap<EncoderKey, AvStream>,
    /// Input reading waits for an explicit `Start` (see `Bus::new_deferred`).
    deferred: bool,
    /// `Start` has been received.
    started: bool,
    /// The input is a live source (see `Bus::add_input_with_live`).
    live: bool,
    /// Shared with `Bus::stop`, which sets it to abort a blocking input open.
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}

impl BusState {
    fn new() -> Self {
        Self {
            input_config: None,
            output_config: HashMap::new(),
            input_task: None,
            pending_input: None,
            input_streams: Vec::new(),
            decoder_tasks: HashMap::new(),
            encoder_tasks: HashMap::new(),
            encoder_output_streams: HashMap::new(),
            input_options: None,
            deferred: false,
            started: false,
            live: false,
            shutdown: Arc::default(),
        }
    }
}

pub type VideoRawFrameStream = Pin<Box<dyn Stream<Item = Option<VideoFrame>> + Send + Sync>>;

pub enum BusCommand {
    AddInput {
        input: InputConfig,
        options: Option<HashMap<String, String>>,
        /// `None`: infer from the input type.
        live: Option<bool>,
        result: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
    RemoveInput {
        result: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
    /// Start reading the input (deferred buses wait for this).
    Start {
        result: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
    AddOutput {
        output: OutputConfig,
        result: tokio::sync::oneshot::Sender<anyhow::Result<(AvStream, VideoRawFrameStream)>>,
    },
    /// Subscribe to the pipe's decoded audio broadcast (ensures the audio
    /// decoder task is running). Receiver yields `RawFrame::Audio` (and may
    /// yield video; filter on the receiving side).
    SubscribeAudio {
        result: tokio::sync::oneshot::Sender<anyhow::Result<crate::frame::RawFrameReceiver>>,
    },
    /// Subscribe to the pipe's decoded video broadcast (ensures the video
    /// decoder task is running). Receiver yields `RawFrame::Video`.
    SubscribeVideo {
        result: tokio::sync::oneshot::Sender<anyhow::Result<crate::frame::RawFrameReceiver>>,
    },
}

pub enum InputConfig {
    Net { url: String },
    File { path: String },
    Device { display: String, format: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputAvType {
    Video,
    Audio,
}

pub struct OutputConfig {
    pub id: String,
    pub dest: OutputDest,
    pub av_type: OutputAvType,
    /// Encode config for the primary (`av_type`) stream. `None` = copy.
    pub encode: Option<EncodeConfig>,
    /// Encode config for the audio stream carried alongside video in File/Net
    /// outputs (`include_audio`). `None` = copy. Independent of `encode`, so a
    /// File/Net output can copy video while transcoding audio, or vice versa.
    pub audio_encode: Option<EncodeConfig>,
    /// When true, include both video and audio streams in File/Net outputs.
    pub include_audio: bool,
}

impl OutputConfig {
    pub fn new(id: String, av_type: OutputAvType, dest: OutputDest) -> Self {
        Self {
            id,
            dest,
            av_type,
            encode: None,
            audio_encode: None,
            include_audio: false,
        }
    }

    pub fn with_encode(mut self, encode: EncodeConfig) -> Self {
        self.encode = Some(encode);
        self
    }

    /// Set the encode config for the included audio stream (File/Net + `with_audio`).
    pub fn with_audio_encode(mut self, encode: EncodeConfig) -> Self {
        self.audio_encode = Some(encode);
        self
    }

    pub fn with_audio(mut self) -> Self {
        self.include_audio = true;
        self
    }
}

pub enum OutputDest {
    ///! Mux to a network stream (no seekable), some times called live streaming
    ///! eg: rtmp://localhost:1935/live/stream
    ///! eg: rtsp://host:8554/path
    ///! format: e.g. "rtsp", "flv" (required for URL-only outputs; None = guess from URL)
    Net { url: String, format: Option<String> },
    /// Mux to a file (seekable). Produces standard MP4 that any player can open.
    File { path: String },
    /// Raw video frames (only support decode, no encoding)
    Raw,
    /// Mux to a stream (no seekable)
    Mux { format: String },
    /// Stream of encoded packets (e.g. for RawPacket sink). Requires encoder.
    Encoded,
    /// Pass demuxed input packets through unmodified — no decoder, no encoder,
    /// no muxer. Each emitted item is exactly one frame as it left the input
    /// demuxer (e.g. raw H.264 NALU bytes for video, raw AAC frame for audio,
    /// without any container framing). Use this when the consumer (e.g.
    /// ZLMediaKit) already knows how to packetise raw codec frames.
    Demuxed,
}

#[derive(Clone, Debug)]
pub struct EncodeConfig {
    // "h264", "hevc", "rawvideo", "aac", "opus"
    pub codec: String,
    // None = keep original
    pub width: Option<u32>,
    // None = keep original
    pub height: Option<u32>,
    // bps (video bitrate)
    pub bitrate: Option<u64>,
    // "ultrafast", "medium", etc.
    pub preset: Option<String>,
    // "yuv420p", "rgb24", etc.
    pub pixel_format: Option<String>,
    // Audio: sample rate (e.g. 44100, 48000)
    pub sample_rate: Option<u32>,
    // Audio: number of channels (e.g. 2)
    pub channels: Option<u32>,
    // Audio: bitrate in bps (e.g. 128000)
    pub audio_bitrate: Option<u64>,
}

impl Default for EncodeConfig {
    fn default() -> Self {
        Self {
            codec: "h264".to_string(),
            width: None,
            height: None,
            bitrate: None,
            preset: None,
            pixel_format: None,
            sample_rate: None,
            channels: None,
            audio_bitrate: None,
        }
    }
}

impl PartialEq for EncodeConfig {
    fn eq(&self, other: &Self) -> bool {
        self.codec == other.codec
            && self.width == other.width
            && self.height == other.height
            && self.bitrate == other.bitrate
            && self.preset == other.preset
            && self.pixel_format == other.pixel_format
            && self.sample_rate == other.sample_rate
            && self.channels == other.channels
            && self.audio_bitrate == other.audio_bitrate
    }
}

impl Eq for EncodeConfig {}

impl std::hash::Hash for EncodeConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.codec.hash(state);
        self.width.hash(state);
        self.height.hash(state);
        self.bitrate.hash(state);
        self.preset.hash(state);
        self.pixel_format.hash(state);
        self.sample_rate.hash(state);
        self.channels.hash(state);
        self.audio_bitrate.hash(state);
    }
}

#[cfg(test)]
#[path = "bus_test.rs"]
mod bus_test;

#[cfg(test)]
#[path = "bus_rtsp_test.rs"]
mod bus_rtsp_test;
