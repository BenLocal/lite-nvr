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
    Eof,
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
}

pub struct Bus {
    id: String,
    cancel: CancellationToken,
    tx: tokio::sync::mpsc::Sender<BusCommand>,
}

impl Bus {
    pub fn new(id: &str) -> Self {
        let id = id.to_string();
        let cancel = CancellationToken::new();
        let (tx, rx) = tokio::sync::mpsc::channel(1024);

        let cancel_clone = cancel.clone();
        tokio::spawn(async move { Self::inner_loop(cancel_clone, rx).await });
        Self { id: id, cancel, tx }
    }

    async fn inner_loop(
        cancel: CancellationToken,
        mut rx: tokio::sync::mpsc::Receiver<BusCommand>,
    ) {
        let cancel_clone = cancel.clone();
        let mut state = BusState::new();
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
                result,
            } => {
                result
                    .send(Self::add_input_internal(state, input, options).await)
                    .map_err(|e| anyhow::anyhow!("send result error: {:#?}", e))?;
            }
            BusCommand::RemoveInput { result } => {
                if let Some(input) = state.input_task.take() {
                    input.stop();
                    drop(input);
                }
                state.pending_input = None;
                state.input_config = None;
                result
                    .send(Ok(()))
                    .map_err(|e| anyhow::anyhow!("send result error: {:#?}", e))?;
            }
            BusCommand::AddOutput { output, result } => {
                let id = &output.id;
                if state.output_config.contains_key(id) {
                    let _ = result.send(Err(anyhow::anyhow!("output already exists")));
                    return Err(anyhow::anyhow!("output already exists"));
                }

                // try to start input task
                if state.input_task.is_none() && state.input_config.is_some() {
                    if let Err(e) = Self::prepare_input_task(state).await {
                        let msg = format!("{:#}", e);
                        let _ = result.send(Err(anyhow::anyhow!("{}", msg)));
                        return Err(anyhow::anyhow!("{}", msg));
                    }
                }
                let input_stream = state
                    .input_streams
                    .iter()
                    .find(|s| match output.av_type {
                        OutputAvType::Video => s.is_video(),
                        OutputAvType::Audio => s.is_audio(),
                    })
                    .ok_or(anyhow::anyhow!("stream not found"))?;
                let input_stream_index = input_stream.index();
                let need_decoder = Self::try_decoder(input_stream, &output)?;
                let need_encoder = Self::try_encoder(input_stream, &output)?;
                let is_file_net = matches!(
                    &output.dest,
                    OutputDest::File { .. } | OutputDest::Net { .. }
                );
                // File/Net decide copy vs transcode per stream and start their
                // decoder/encoder tasks inside the muxer builder; every other
                // dest starts the primary stream's tasks here.
                if !is_file_net {
                    if need_decoder {
                        Self::start_decoder_task(state, input_stream_index).await?;
                    }
                    if need_encoder {
                        Self::start_encoder_task(state, input_stream_index, output.encode.as_ref())
                            .await?;
                    }
                }

                let stream_result = match &output.dest {
                    OutputDest::Raw => {
                        Self::create_decoder_raw_output_stream(state, input_stream_index).await
                    }
                    OutputDest::File { path } => {
                        Self::create_mux_to_file(state, path, input_stream_index, &output).await
                    }
                    OutputDest::Net { url, format } => {
                        Self::create_mux_to_net(
                            state,
                            url,
                            format.as_deref(),
                            input_stream_index,
                            &output,
                        )
                        .await
                    }
                    OutputDest::Mux { format } => {
                        if need_encoder {
                            Self::create_mux_output_stream_from_encoder(
                                state,
                                format,
                                input_stream_index,
                            )
                            .await
                        } else {
                            Self::create_mux_output_stream(state, format, input_stream_index).await
                        }
                    }
                    OutputDest::Encoded => {
                        Self::create_encoded_output_stream(state, input_stream_index).await
                    }
                    OutputDest::Demuxed => {
                        Self::create_demuxed_output_stream(state, input_stream_index).await
                    }
                };

                match stream_result {
                    Ok((av, stream)) => {
                        state.output_config.insert(id.clone(), output);
                        if let Err(e) = Self::start_input_task(state).await {
                            let msg = format!("{:#}", e);
                            let _ = result.send(Err(anyhow::anyhow!("{}", msg)));
                            return Err(anyhow::anyhow!("{}", msg));
                        }
                        result
                            .send(Ok((av, stream)))
                            .map_err(|_| anyhow::anyhow!("send result error: receiver dropped"))?;
                    }
                    Err(e) => {
                        let msg = format!("{:#}", e);
                        let _ = result.send(Err(anyhow::anyhow!("{}", msg)));
                        return Err(anyhow::anyhow!("{}", msg));
                    }
                }
            }
        }

        Ok(())
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
        let plan = Self::build_mux_plan(state, primary_index, output)?;
        Self::start_mux_transcoders(state, &plan).await?;
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
        let plan = Self::build_mux_plan(state, primary_index, output)?;
        Self::start_mux_transcoders(state, &plan).await?;
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
    /// Audio transcode is not yet implemented, so audio is always copied.
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
        let wants_transcode = encode.is_some_and(|e| Self::encode_needed(stream, e));

        // Phase 1: only video transcode is wired; audio is always copied.
        if wants_transcode && !stream.is_video() {
            log::warn!(
                "audio transcode not yet supported; copying audio stream {}",
                stream.index()
            );
        }
        let transcode = wants_transcode && stream.is_video();

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
        }
    }

    /// Start a decoder + encoder task for each transcoded stream in the plan.
    async fn start_mux_transcoders(
        state: &mut BusState,
        plan: &[MuxPlanEntry],
    ) -> anyhow::Result<()> {
        for entry in plan.iter().filter(|e| e.transcode) {
            Self::start_decoder_task(state, entry.input_index).await?;
            Self::start_encoder_task(state, entry.input_index, entry.encode.as_ref()).await?;
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
        let (mut output, label) = match &target {
            MuxTarget::File(path) => (AvOutput::new(path, None, None)?, path.clone()),
            MuxTarget::Net { url, format } => {
                // RTSP output often needs rtsp_transport=tcp for avio_open2.
                let options = match format.as_deref() {
                    Some("rtsp") => {
                        let mut opts = Dictionary::new();
                        opts.set("rtsp_transport", "tcp");
                        Some(opts)
                    }
                    _ => None,
                };
                (
                    AvOutput::new(url, format.as_deref(), options).map_err(|e| {
                        anyhow::anyhow!("mux AvOutput::new(url={:?}): {:?}", url, e)
                    })?,
                    url.clone(),
                )
            }
        };

        // Add one output stream per planned stream; collect the packet sources.
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
            let out_stream = if entry.transcode {
                AvStream::for_encoder_output(&input_stream, entry.codec_id)
                    .with_index(entry.input_index)
            } else {
                input_stream
            };
            output.add_stream(&out_stream)?;
            if primary_av.is_none() {
                primary_av = Some(out_stream.clone());
            }
            if entry.transcode {
                let recv = state
                    .encoder_tasks
                    .get(&entry.input_index)
                    .ok_or(anyhow::anyhow!("encoder task not found"))?
                    .subscribe();
                enc_receivers.push((entry.input_index, recv));
            } else {
                copied_indices.insert(entry.input_index);
            }
        }
        let primary_av = primary_av.ok_or(anyhow::anyhow!("mux plan is empty"))?;

        let input_receiver = state
            .input_task
            .as_ref()
            .ok_or(anyhow::anyhow!("input task not found"))?
            .subscribe();

        tokio::spawn(async move {
            // One MuxSignal stream per source. A source's channel may stay open
            // after its logical end (the input/encoder tasks keep a sender), so
            // termination is driven by the EOF *signal* (one per source), not by
            // channel close.
            let mut sources: Vec<Pin<Box<dyn Stream<Item = MuxSignal> + Send>>> = Vec::new();
            let copied = Arc::new(copied_indices);
            {
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
                            Err(_) => None, // Lagged / Closed
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
                        Err(_) => None,
                    }
                });
                sources.push(Box::pin(s));
            }

            let total_sources = sources.len();
            let mut eofs = 0usize;
            let mut merged = futures::stream::select_all(sources);
            let mut output = output;
            while let Some(sig) = merged.next().await {
                match sig {
                    MuxSignal::Packet(idx, packet) => {
                        if let Err(e) = output.write_packet(idx, packet) {
                            log::error!("mux write_packet error: {:#?}", e);
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
            if let Err(e) = output.finish() {
                log::error!(
                    "mux finish error: {:#?}\nbacktrace:\n{}",
                    e,
                    Backtrace::capture()
                );
            }
            log::info!("mux finished: {}", label);
        });

        Ok((
            primary_av,
            Box::pin(futures::stream::empty::<Option<VideoFrame>>()),
        ))
    }

    async fn create_encoded_output_stream(
        state: &mut BusState,
        input_stream_index: usize,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let av = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("stream not found"))?;
        let encoder_receiver = state
            .encoder_tasks
            .get(&input_stream_index)
            .ok_or(anyhow::anyhow!("encoder task not found"))?
            .subscribe();

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
        input_stream_index: usize,
    ) -> anyhow::Result<(AvStream, VideoRawFrameStream)> {
        let mut encoder_receiver = state
            .encoder_tasks
            .get(&input_stream_index)
            .ok_or(anyhow::anyhow!("encoder task not found"))?
            .subscribe();

        let input_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("no matching stream in input"))?;

        let codec_id = match format {
            "h264" => ffmpeg_next::codec::Id::H264,
            "hevc" | "h265" => ffmpeg_next::codec::Id::HEVC,
            "aac" | "adts" => ffmpeg_next::codec::Id::AAC,
            "opus" => ffmpeg_next::codec::Id::OPUS,
            _ => {
                return Err(anyhow::anyhow!(
                    "unsupported mux format for encoder output: {}",
                    format
                ));
            }
        };
        let encoder_output_stream = AvStream::for_encoder_output(input_stream, codec_id);

        let mut stream = AvOutputStream::new(format)?;
        stream.add_stream(&encoder_output_stream)?;
        let (writer, reader) = stream.into_split();

        tokio::spawn(async move {
            let mut writer = writer;
            loop {
                match encoder_receiver.recv().await {
                    Ok(cmd) => match cmd {
                        RawPacketCmd::Data(mut packet) => {
                            packet.get_mut().set_stream(0);
                            if let Err(e) = writer.write_packet(packet) {
                                log::error!("mux write_packet error: {}", e.to_string());
                            }
                        }
                        RawPacketCmd::EOF => break,
                    },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("mux encoder_receiver lagged, dropped {} messages", n);
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
        });

        Ok((
            encoder_output_stream.clone(),
            Box::pin(reader.map(|pkg| Some(VideoFrame::from(pkg)))),
        ))
    }

    async fn create_mux_output_stream(
        state: &mut BusState,
        format: &str,
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
            .ok_or(anyhow::anyhow!("no matching stream in input"))?;
        let target_stream_index = target_stream.index();
        let mut stream = AvOutputStream::new(format)?;
        stream.add_stream(&target_stream)?;
        let (writer, reader) = stream.into_split();

        tokio::spawn(async move {
            let mut writer = writer;
            loop {
                match input_receiver.recv().await {
                    Ok(RawPacketCmd::Data(packet)) => {
                        if packet.index() == target_stream_index {
                            if let Err(e) = writer.write_packet(packet) {
                                log::error!("mux write_packet error: {}", e.to_string());
                            }
                        }
                    }
                    Ok(RawPacketCmd::EOF) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("mux input_receiver lagged, dropped {} messages", n);
                        continue;
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
        });

        Ok((
            target_stream.clone(),
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
        let stream = BroadcastStream::new(
            state
                .decoder_tasks
                .get(&stream_index)
                .ok_or(anyhow::anyhow!("decoder task not found"))?
                .subscribe(),
        )
        .map(|cmd| match cmd {
            Ok(cmd) => match cmd {
                RawFrameCmd::Data(frame) => Some(VideoFrame::try_from(frame).unwrap()),
                RawFrameCmd::EOF => None,
            },
            Err(e) => {
                log::error!(
                    "decoder task error: {:#?}\nbacktrace:\n{}",
                    e,
                    Backtrace::capture()
                );
                None
            }
        });

        Ok((av.clone(), Box::pin(stream)))
    }

    async fn add_input_internal(
        state: &mut BusState,
        input: InputConfig,
        options: Option<HashMap<String, String>>,
    ) -> anyhow::Result<()> {
        if state.input_config.is_some() {
            return Err(anyhow::anyhow!("input already exists"));
        } else {
            state.input_config = Some(input);
            state.input_options = options;
        }

        if !state.output_config.is_empty() && state.input_task.is_none() {
            Self::prepare_input_task(state).await?;
            Self::start_input_task(state).await?;
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

    async fn start_encoder_task(
        state: &mut BusState,
        input_stream_index: usize,
        encode: Option<&EncodeConfig>,
    ) -> anyhow::Result<()> {
        let input_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("stream not found"))?;
        if state.encoder_tasks.contains_key(&input_stream_index) {
            return Ok(());
        }

        // Audio encoder path
        if input_stream.is_audio() {
            let encoder_task = EncoderTask::new();
            let encoder_receiver = state
                .decoder_tasks
                .get(&input_stream_index)
                .ok_or(anyhow::anyhow!("decoder task not found for audio stream"))?
                .subscribe();
            let audio_settings = Self::audio_settings_from_config(encode);
            let encoder = Encoder::new_audio(input_stream, audio_settings, None)?;
            encoder_task.start(encoder, encoder_receiver).await;
            state.encoder_tasks.insert(input_stream_index, encoder_task);
            return Ok(());
        }

        // Video encoder path
        let codec_id = input_stream.parameters().id();
        let encoder_task = EncoderTask::new();
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
            let encoder_opts = Self::encoder_options_from_config(encode);
            let encoder = Encoder::new(input_stream, encoder_settings, encoder_opts)?;
            // Spawn task: packet -> frame conversion, then forward to encoder
            {
                let mut packet_rx = packet_receiver;
                let frame_tx = frame_tx;
                tokio::spawn(async move {
                    loop {
                        match packet_rx.recv().await {
                            Ok(RawPacketCmd::Data(packet)) => {
                                if let Ok(frame) =
                                    packet_to_raw_video_frame(packet, width, height, pixel_format)
                                {
                                    let _ = frame_tx.send(RawFrameCmd::Data(frame));
                                }
                            }
                            Ok(RawPacketCmd::EOF) => {
                                let _ = frame_tx.send(RawFrameCmd::EOF);
                                break;
                            }
                            Err(_) => continue,
                        }
                    }
                });
            }
            encoder_task.start(encoder, frame_rx).await;
        } else {
            let encoder_receiver = state
                .decoder_tasks
                .get(&input_stream_index)
                .ok_or(anyhow::anyhow!("decoder task not found"))?
                .subscribe();
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
            let encoder_opts = Self::encoder_options_from_config(encode);
            let encoder = Encoder::new(input_stream, encoder_settings, encoder_opts)?;
            encoder_task.start(encoder, encoder_receiver).await;
        }

        state.encoder_tasks.insert(input_stream_index, encoder_task);
        Ok(())
    }

    async fn start_decoder_task(
        state: &mut BusState,
        input_stream_index: usize,
    ) -> anyhow::Result<()> {
        let input_stream = state
            .input_streams
            .iter()
            .find(|s| s.index() == input_stream_index)
            .ok_or(anyhow::anyhow!("stream not found"))?;
        if state.decoder_tasks.contains_key(&input_stream_index) {
            return Ok(());
        }
        let codec_id = input_stream.parameters().id();
        if codec_id == ffmpeg_next::codec::Id::RAWVIDEO {
            return Ok(());
        }
        let decoder_receiver = state
            .input_task
            .as_ref()
            .ok_or(anyhow::anyhow!("input task not found"))?
            .subscribe();
        let decoder = Decoder::new(input_stream)?;
        let decoder_task = DecoderTask::new();
        decoder_task.start(decoder, decoder_receiver).await;
        state.decoder_tasks.insert(input_stream_index, decoder_task);

        Ok(())
    }

    async fn prepare_input_task(state: &mut BusState) -> anyhow::Result<()> {
        if state.input_task.is_some() {
            return Ok(());
        }
        let options = state.input_options.as_ref().map(|options| {
            ffmpeg_next::Dictionary::from_iter(
                options.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            )
        });
        let input = match state.input_config.as_ref() {
            Some(InputConfig::Net { url }) => AvInput::new(url, None, options)?,
            Some(InputConfig::File { path }) => AvInput::new(path, None, options)?,
            Some(InputConfig::Device { display, format }) => {
                AvInput::new(display, Some(format), options)?
            }
            None => return Err(anyhow::anyhow!("input config is not set")),
        };

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

    pub async fn add_input(
        &self,
        input: InputConfig,
        options: Option<HashMap<String, String>>,
    ) -> anyhow::Result<()> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(BusCommand::AddInput {
                input,
                options,
                result: tx,
            })
            .await?;
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

    pub fn stop(&self) {
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
    encoder_tasks: HashMap<usize, EncoderTask>,
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
            input_options: None,
        }
    }
}

pub type VideoRawFrameStream = Pin<Box<dyn Stream<Item = Option<VideoFrame>> + Send + Sync>>;

pub enum BusCommand {
    AddInput {
        input: InputConfig,
        options: Option<HashMap<String, String>>,
        result: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
    RemoveInput {
        result: tokio::sync::oneshot::Sender<anyhow::Result<()>>,
    },
    AddOutput {
        output: OutputConfig,
        result: tokio::sync::oneshot::Sender<anyhow::Result<(AvStream, VideoRawFrameStream)>>,
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
