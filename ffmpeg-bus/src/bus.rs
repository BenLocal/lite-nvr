use std::{backtrace::Backtrace, collections::HashMap, hash::Hasher, pin::Pin};

use futures::{Stream, StreamExt};
use log::error;
use tokio_stream::wrappers::BroadcastStream;
use tokio_util::sync::CancellationToken;

use ffmpeg_next::Dictionary;

use crate::{
    decoder::{Decoder, DecoderTask},
    encoder::{Encoder, EncoderTask, Settings, pixel_format_for_libx264},
    frame::{RawFrameCmd, VideoFrame, packet_to_raw_video_frame},
    input::{AvInput, AvInputTask},
    output::{AvOutput, AvOutputStream},
    packet::RawPacketCmd,
    stream::AvStream,
};

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

                // Phase 1: prepare input (open file, create broadcast channel) but do NOT
                // start reading packets yet — subscribers must be registered first.
                let deferred_input = if state.input_task.is_none() && state.input_config.is_some() {
                    match Self::prepare_input_task(state).await {
                        Ok(input) => Some(input),
                        Err(e) => {
                            let msg = format!("{:#}", e);
                            let _ = result.send(Err(anyhow::anyhow!("{}", msg)));
                            return Err(anyhow::anyhow!("{}", msg));
                        }
                    }
                } else {
                    None
                };

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
                if need_decoder {
                    Self::start_decoder_task(state, input_stream_index).await?;
                }
                if need_encoder {
                    Self::start_encoder_task(state, input_stream_index, output.encode.as_ref())
                        .await?;
                }

                let stream_result = match &output.dest {
                    OutputDest::Raw => {
                        Self::create_decoder_raw_output_stream(state, input_stream_index).await
                    }
                    OutputDest::File { path } => {
                        Self::create_mux_to_file(state, path, input_stream_index).await
                    }
                    OutputDest::Net { url, format } => {
                        Self::create_mux_to_net(state, url, format.as_deref(), input_stream_index)
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
                };

                // Phase 2: NOW start reading packets — all subscribers are registered.
                if let Some(input) = deferred_input {
                    Self::begin_input_reading(state, input).await;
                }

                match stream_result {
                    Ok((av, stream)) => {
                        state.output_config.insert(id.clone(), output);
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
            OutputDest::Mux { .. } => Ok(input_codec == ffmpeg_next::codec::Id::WRAPPED_AVFRAME
                || Self::try_encoder(input_stream, output).unwrap_or(false)),
            OutputDest::Net { .. } => Ok(true),
            OutputDest::Encoded => Ok(true),
        }
    }

    fn try_encoder(input_stream: &AvStream, output: &OutputConfig) -> anyhow::Result<bool> {
        let input_codec = input_stream.parameters().id();

        if let OutputDest::Raw = output.dest {
            return Ok(false);
        }

        if input_codec == ffmpeg_next::codec::Id::RAWVIDEO
            || input_codec == ffmpeg_next::codec::Id::WRAPPED_AVFRAME
        {
            return Ok(true);
        }

        if let OutputDest::Encoded = output.dest {
            return Ok(true);
        }

        // Mux format "h264" (or similar) requires encoded packets; use encoder when input is not already that codec
        if let OutputDest::Mux { format } = &output.dest {
            let need_encode = match format.as_str() {
                "h264" => input_codec != ffmpeg_next::codec::Id::H264,
                "hevc" | "h265" => input_codec != ffmpeg_next::codec::Id::HEVC,
                _ => false,
            };
            if need_encode {
                return Ok(true);
            }
        }

        if output.encode.is_some() {
            return Ok(true);
        }

        Ok(false)
    }

    /// Mux to a real file path (seekable). Produces standard MP4 that any player can open.
    async fn create_mux_to_file(
        state: &mut BusState,
        path: &str,
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
        let path_owned = path.to_string();

        let mut output = AvOutput::new(path, None, None)?;
        output.add_stream(&target_stream)?;

        tokio::spawn(async move {
            let mut output = output;
            loop {
                let cmd = match input_receiver.recv().await {
                    Ok(cmd) => cmd,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("mux_to_file lagged by {} packets", n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                match cmd {
                    RawPacketCmd::Data(packet) => {
                        if packet.index() == target_stream_index {
                            if let Err(e) = output.write_packet(target_stream_index, packet) {
                                log::error!(
                                    "mux to file write_packet error: {:#?}\nbacktrace:\n{}",
                                    e,
                                    Backtrace::capture()
                                );
                            }
                        }
                    }
                    RawPacketCmd::EOF => break,
                }
            }
            if let Err(e) = output.finish() {
                log::error!(
                    "mux to file finish error: {:#?}\nbacktrace:\n{}",
                    e,
                    Backtrace::capture()
                );
            }
            log::info!("mux to file finished: {}", path_owned);
        });

        Ok((
            target_stream.clone(),
            Box::pin(futures::stream::empty::<Option<VideoFrame>>()),
        ))
    }

    /// Mux to a network URL (e.g. rtmp://, rtsp://). Remux only (input packets).
    /// format: e.g. Some("rtsp"), Some("flv"); None = let FFmpeg guess from URL.
    async fn create_mux_to_net(
        state: &mut BusState,
        url: &str,
        format: Option<&str>,
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
        let url_owned = url.to_string();

        // RTSP output often needs rtsp_transport=tcp for avio_open2 to succeed
        let options = match format {
            Some("rtsp") => {
                let mut opts = Dictionary::new();
                opts.set("rtsp_transport", "tcp");
                Some(opts)
            }
            _ => None,
        };

        let mut output = AvOutput::new(url, format, options).map_err(|e| {
            anyhow::anyhow!(
                "create_mux_to_net AvOutput::new(url={:?}, format={:?}): {:?}",
                url,
                format,
                e
            )
        })?;
        output
            .add_stream(&target_stream)
            .map_err(|e| anyhow::anyhow!("create_mux_to_net add_stream: {:?}", e))?;

        tokio::spawn(async move {
            let mut output = output;
            loop {
                let cmd = match input_receiver.recv().await {
                    Ok(cmd) => cmd,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("mux_to_net lagged by {} packets", n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                match cmd {
                    RawPacketCmd::Data(packet) => {
                        if packet.index() == target_stream_index {
                            if let Err(e) = output.write_packet(target_stream_index, packet) {
                                log::error!(
                                    "mux to net write_packet error: {:#?}\nbacktrace:\n{}",
                                    e,
                                    Backtrace::capture()
                                );
                            }
                        }
                    }
                    RawPacketCmd::EOF => break,
                }
            }
            if let Err(e) = output.finish() {
                log::error!(
                    "mux to net finish error: {:#?}\nbacktrace:\n{}",
                    e,
                    Backtrace::capture()
                );
            }
            log::info!("mux to net finished: {}", url_owned);
        });

        Ok((
            target_stream.clone(),
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
                let cmd = match input_receiver.recv().await {
                    Ok(cmd) => cmd,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("mux_output_stream lagged by {} packets", n);
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                match cmd {
                    RawPacketCmd::Data(packet) => {
                        if packet.index() == target_stream_index {
                            if let Err(e) = writer.write_packet(packet) {
                                log::error!("mux write_packet error: {}", e.to_string());
                            }
                        }
                    }
                    RawPacketCmd::EOF => break,
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
            let input = Self::prepare_input_task(state).await?;
            Self::begin_input_reading(state, input).await;
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

        let codec_id = input_stream.parameters().id();
        let encoder_task = EncoderTask::new();
        // Only RAWVIDEO has raw pixel data in packets; use packet->frame conversion.
        // WRAPPED_AVFRAME packets wrap AVFrame (not raw pixels), so use decoder path.
        if codec_id == ffmpeg_next::codec::Id::RAWVIDEO {
            let (width, height, pixel_format) =
                Self::raw_video_params_from_parameters(input_stream.parameters());
            let (width, height) = Self::ensure_video_dimensions(width, height);
            let encoder_settings = Settings {
                width,
                height,
                pixel_format: pixel_format_for_libx264(pixel_format),
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
            let encoder_settings = if codec_id == ffmpeg_next::codec::Id::WRAPPED_AVFRAME {
                let (width, height, pixel_format) =
                    Self::raw_video_params_from_parameters(input_stream.parameters());
                let (width, height) = Self::ensure_video_dimensions(width, height);
                Settings {
                    width,
                    height,
                    pixel_format: pixel_format_for_libx264(pixel_format),
                    codec: Some("libx264".to_string()),
                    ..Settings::default()
                }
            } else {
                Settings {
                    codec: Some("libx264".to_string()),
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

    /// Phase 1: Open input, populate streams, create AvInputTask (broadcast channel ready
    /// for subscribers), but do NOT start reading packets yet.
    /// Returns the AvInput that must be passed to `begin_input_reading` later.
    async fn prepare_input_task(state: &mut BusState) -> anyhow::Result<AvInput> {
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
        println!("start add input streams: ");
        for (index, stream) in streams {
            println!(
                "stream index: {}, stream id: {:#?}, time_base: {:#?}",
                index,
                stream.parameters().id(),
                stream.time_base()
            );
            state.input_streams.push(stream.clone());
        }

        state.input_task = Some(AvInputTask::new());

        Ok(input)
    }

    /// Phase 2: Start actually reading packets from the input.
    /// Call this AFTER all subscribers (decoder, encoder, mux) have been registered.
    async fn begin_input_reading(state: &BusState, input: AvInput) {
        if let Some(task) = state.input_task.as_ref() {
            task.start(input).await;
        }
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
    pub encode: Option<EncodeConfig>,
}

impl OutputConfig {
    pub fn new(id: String, av_type: OutputAvType, dest: OutputDest) -> Self {
        Self {
            id,
            dest,
            av_type,
            encode: None,
        }
    }

    pub fn with_encode(mut self, encode: EncodeConfig) -> Self {
        self.encode = Some(encode);
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
}

#[derive(Clone, Debug)]
pub struct EncodeConfig {
    // "h264", "hevc", "rawvideo"
    pub codec: String,
    // None = keep original
    pub width: Option<u32>,
    // None = keep original
    pub height: Option<u32>,
    // bps
    pub bitrate: Option<u64>,
    // "ultrafast", "medium", etc.
    pub preset: Option<String>,
    // "yuv420p", "rgb24", etc.
    pub pixel_format: Option<String>,
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
    }
}

#[cfg(test)]
#[path = "bus_test.rs"]
mod bus_test;
