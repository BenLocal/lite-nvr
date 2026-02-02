//! 优化架构：Input 输出两种数据 + 相同编码共享 Encoder
//!
//! 数据流：
//! ```text
//!                                 ┌─► RawPacket broadcast ─► [Remux outputs] (无需重编码)
//!                                 │
//! Input (Demux) ──► decode? ──────┤
//!                                 │
//!                                 └─► DecodedFrame broadcast ─► [EncodeTask per config]
//!                                                                      │
//!                                                          ┌───────────┴───────────┐
//!                                                          ▼                       ▼
//!                                                   EncodedPacket             EncodedPacket
//!                                                          │                       │
//!                                                    [Mux outputs]           [Mux outputs]
//! ```
//!
//! 优化点：
//! 1. 不需要重编码的输出直接用 RawPacket（remux）
//! 2. 需要重编码但配置相同的输出共享一个 Encoder
//! 3. DecodedFrame 可供 Raw sink 消费

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::pipe::RawSinkSource;

// ============================================================================
// 数据类型
// ============================================================================

/// 原始编码包（demux 后、decode 前）
#[derive(Clone, Debug)]
pub struct RawPacket {
    pub stream_index: usize,
    pub data: Vec<u8>,
    pub pts: i64,
    pub dts: i64,
    pub is_key: bool,
    pub codec_id: i32, // AVCodecID
}

/// 解码后的视频帧
#[derive(Clone, Debug)]
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub format: i32, // AVPixelFormat
    pub data: Vec<u8>,
    pub linesize: [i32; 4],
    pub pts: i64,
}

/// 编码后的包（encode 后）
#[derive(Clone, Debug)]
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub pts: i64,
    pub dts: i64,
    pub is_key: bool,
}

// ============================================================================
// 配置类型
// ============================================================================

/// 编码配置（用作 HashMap key，相同配置共享 encoder）
#[derive(Clone, Debug)]
pub struct EncodeConfig {
    pub codec: String,          // "h264", "hevc", "rawvideo"
    pub width: Option<u32>,     // None = 保持原始
    pub height: Option<u32>,
    pub bitrate: Option<u64>,   // bps
    pub preset: Option<String>, // "ultrafast", "medium", etc.
    pub pixel_format: Option<String>, // "yuv420p", "rgb24", etc.
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

impl Hash for EncodeConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.codec.hash(state);
        self.width.hash(state);
        self.height.hash(state);
        self.bitrate.hash(state);
        self.preset.hash(state);
        self.pixel_format.hash(state);
    }
}

/// 输出目标
#[derive(Clone)]
pub enum OutputDest {
    /// 网络推流 (RTSP/RTMP/HLS...)
    Network { url: String, format: String },
    /// 原始帧数据 sink
    RawFrame { sink: Arc<RawSinkSource> },
    /// 编码后的包 sink
    RawPacket { sink: Arc<RawSinkSource> },
}

/// 单个输出的配置
#[derive(Clone)]
pub struct OutputConfig {
    pub dest: OutputDest,
    /// None = 直接 remux（不重编码），Some = 使用指定编码
    pub encode: Option<EncodeConfig>,
}

/// 输入配置
#[derive(Clone)]
pub enum InputConfig {
    Network { url: String },
}

/// Pipeline 配置
pub struct PipeV2Config {
    pub input: InputConfig,
    pub outputs: Vec<OutputConfig>,
}

// ============================================================================
// Pipeline 实现
// ============================================================================

/// V2 Pipeline：优化的解码+编码架构
pub struct PipeV2 {
    config: PipeV2Config,
    cancel: CancellationToken,
    started: AtomicBool,
}

impl PipeV2 {
    pub fn new(config: PipeV2Config) -> Self {
        Self {
            config,
            cancel: CancellationToken::new(),
            started: AtomicBool::new(false),
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// 启动 pipeline
    pub async fn start(&self) {
        if self.started.swap(true, Ordering::Relaxed) {
            log::warn!("PipeV2 already started");
            return;
        }

        let input_url = match &self.config.input {
            InputConfig::Network { url } => url.clone(),
        };

        // 分析 outputs，决定需要哪些 channels
        let analysis = self.analyze_outputs();
        log::info!(
            "PipeV2: need_decode={}, need_raw_packet={}, encode_configs={:?}",
            analysis.need_decode,
            analysis.need_raw_packet,
            analysis.encode_groups.keys().collect::<Vec<_>>()
        );

        // 创建 channels
        let (raw_packet_tx, _) = broadcast::channel::<RawPacket>(64);
        let (decoded_frame_tx, _) = broadcast::channel::<DecodedFrame>(32);

        // 1. 启动 Demux + Decode Task
        let demux_cancel = self.cancel.clone();
        let demux_raw_tx = raw_packet_tx.clone();
        let demux_frame_tx = if analysis.need_decode {
            Some(decoded_frame_tx.clone())
        } else {
            None
        };
        tokio::task::spawn_blocking(move || {
            run_demux_decode_task(&input_url, demux_raw_tx, demux_frame_tx, demux_cancel);
        });

        // 2. 为每个 EncodeConfig 启动一个 Encoder Task
        let mut encode_packet_txs: HashMap<EncodeConfig, broadcast::Sender<EncodedPacket>> =
            HashMap::new();

        for (encode_config, _outputs) in &analysis.encode_groups {
            let (encoded_tx, _) = broadcast::channel::<EncodedPacket>(32);
            encode_packet_txs.insert(encode_config.clone(), encoded_tx.clone());

            let frame_rx = decoded_frame_tx.subscribe();
            let cancel = self.cancel.clone();
            let config = encode_config.clone();
            tokio::task::spawn_blocking(move || {
                run_encode_task(config, frame_rx, encoded_tx, cancel);
            });
        }

        // 3. 为每个 Output 启动 Mux Task
        for output_config in &self.config.outputs {
            let cancel = self.cancel.clone();

            match &output_config.encode {
                None => {
                    // 直接 remux：订阅 raw packet
                    let rx = raw_packet_tx.subscribe();
                    let dest = output_config.dest.clone();
                    tokio::spawn(async move {
                        run_remux_task(dest, rx, cancel).await;
                    });
                }
                Some(encode_config) => {
                    // 需要重编码：订阅对应 encoder 的输出
                    if let Some(encoded_tx) = encode_packet_txs.get(encode_config) {
                        let rx = encoded_tx.subscribe();
                        let dest = output_config.dest.clone();
                        tokio::spawn(async move {
                            run_mux_task(dest, rx, cancel).await;
                        });
                    }
                }
            }
        }

        // 4. 如果有 RawFrame 输出，直接订阅 decoded frames
        for output_config in &self.config.outputs {
            if let OutputDest::RawFrame { sink } = &output_config.dest {
                let rx = decoded_frame_tx.subscribe();
                let sink = sink.clone();
                let cancel = self.cancel.clone();
                tokio::spawn(async move {
                    run_raw_frame_sink_task(sink, rx, cancel).await;
                });
            }
        }
    }

    /// 分析 outputs，决定需要哪些处理流程
    fn analyze_outputs(&self) -> OutputAnalysis {
        let mut need_decode = false;
        let mut need_raw_packet = false;
        let mut encode_groups: HashMap<EncodeConfig, Vec<usize>> = HashMap::new();

        for (i, output) in self.config.outputs.iter().enumerate() {
            // RawFrame 输出需要解码
            if matches!(output.dest, OutputDest::RawFrame { .. }) {
                need_decode = true;
            }

            match &output.encode {
                None => {
                    // 直接 remux 需要 raw packet
                    need_raw_packet = true;
                }
                Some(config) => {
                    // 需要重编码 = 需要解码
                    need_decode = true;
                    encode_groups
                        .entry(config.clone())
                        .or_default()
                        .push(i);
                }
            }
        }

        OutputAnalysis {
            need_decode,
            need_raw_packet,
            encode_groups,
        }
    }
}

struct OutputAnalysis {
    need_decode: bool,
    need_raw_packet: bool,
    /// EncodeConfig -> output indices
    encode_groups: HashMap<EncodeConfig, Vec<usize>>,
}

// ============================================================================
// Task 实现
// ============================================================================

/// Demux + 可选 Decode 任务
fn run_demux_decode_task(
    input_url: &str,
    raw_packet_tx: broadcast::Sender<RawPacket>,
    decoded_frame_tx: Option<broadcast::Sender<DecodedFrame>>,
    cancel: CancellationToken,
) {
    log::info!("DemuxDecodeTask: starting for {}", input_url);

    // TODO: ffmpeg-next 实现
    // 1. let mut ictx = ffmpeg_next::format::input(&input_url)?;
    // 2. 找到 video stream: ictx.streams().best(Type::Video)
    // 3. 创建 decoder: ffmpeg_next::codec::context::Context::from_parameters(stream.parameters())?
    //    let decoder = codec_ctx.decoder().video()?;
    // 4. loop:
    //    for (stream, packet) in ictx.packets() {
    //        if cancel.is_cancelled() { break; }
    //        
    //        // 广播 raw packet
    //        let raw = RawPacket { ... };
    //        let _ = raw_packet_tx.send(raw);
    //        
    //        // 如果需要解码
    //        if let Some(ref frame_tx) = decoded_frame_tx {
    //            decoder.send_packet(&packet)?;
    //            let mut frame = ffmpeg_next::frame::Video::empty();
    //            while decoder.receive_frame(&mut frame).is_ok() {
    //                let decoded = DecodedFrame { ... };
    //                let _ = frame_tx.send(decoded);
    //            }
    //        }
    //    }

    let _ = (input_url, raw_packet_tx, decoded_frame_tx, cancel);
    log::warn!("DemuxDecodeTask: not implemented, use ffmpeg-next format/decoder API");
}

/// Encode 任务：从 decoded frames 编码到 encoded packets
fn run_encode_task(
    config: EncodeConfig,
    mut frame_rx: broadcast::Receiver<DecodedFrame>,
    packet_tx: broadcast::Sender<EncodedPacket>,
    cancel: CancellationToken,
) {
    log::info!("EncodeTask: starting with config {:?}", config);

    // TODO: ffmpeg-next 实现
    // 1. 创建 encoder:
    //    let codec = ffmpeg_next::encoder::find_by_name(&config.codec)?;
    //    let mut encoder = codec.video()?;
    //    encoder.set_width(config.width.unwrap_or(1920));
    //    encoder.set_height(config.height.unwrap_or(1080));
    //    encoder.set_format(Pixel::YUV420P);
    //    encoder.set_time_base((1, 30));
    //    if let Some(preset) = &config.preset { encoder.set_preset(&preset); }
    //    let encoder = encoder.open()?;
    //
    // 2. loop:
    //    loop {
    //        if cancel.is_cancelled() { break; }
    //        match frame_rx.blocking_recv() {
    //            Ok(frame) => {
    //                // 转换 DecodedFrame 到 ffmpeg Video frame
    //                let mut video_frame = ffmpeg_next::frame::Video::new(format, w, h);
    //                video_frame.data_mut(0).copy_from_slice(&frame.data);
    //                video_frame.set_pts(Some(frame.pts));
    //                
    //                encoder.send_frame(&video_frame)?;
    //                let mut packet = ffmpeg_next::Packet::empty();
    //                while encoder.receive_packet(&mut packet).is_ok() {
    //                    let encoded = EncodedPacket { ... };
    //                    let _ = packet_tx.send(encoded);
    //                }
    //            }
    //            Err(broadcast::error::RecvError::Closed) => break,
    //            Err(broadcast::error::RecvError::Lagged(_)) => continue,
    //        }
    //    }

    let _ = (config, frame_rx, packet_tx, cancel);
    log::warn!("EncodeTask: not implemented, use ffmpeg-next encoder API");
}

/// Remux 任务：直接转封装，不重编码
async fn run_remux_task(
    dest: OutputDest,
    mut rx: broadcast::Receiver<RawPacket>,
    cancel: CancellationToken,
) {
    log::info!("RemuxTask: starting for {:?}", dest_name(&dest));

    // TODO: ffmpeg-next 实现
    // 1. 创建 output format context
    // 2. 复制 stream 参数
    // 3. loop: 接收 packet，写入 output

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(packet) => {
                        // TODO: 写入 output
                        log::trace!("RemuxTask: received packet pts={}", packet.pts);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        log::info!("RemuxTask: channel closed");
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("RemuxTask: lagged {} messages", n);
                    }
                }
            }
            _ = cancel.cancelled() => {
                log::info!("RemuxTask: cancelled");
                break;
            }
        }
    }
}

/// Mux 任务：接收编码后的 packets，写入输出
async fn run_mux_task(
    dest: OutputDest,
    mut rx: broadcast::Receiver<EncodedPacket>,
    cancel: CancellationToken,
) {
    log::info!("MuxTask: starting for {:?}", dest_name(&dest));

    // TODO: ffmpeg-next 实现
    // 1. 创建 output format context (rtsp/file/etc)
    // 2. 添加 stream
    // 3. write_header
    // 4. loop: 接收 encoded packet，write_interleaved

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(packet) => {
                        // TODO: 写入 output
                        log::trace!("MuxTask: received encoded packet pts={}", packet.pts);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        log::info!("MuxTask: channel closed");
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("MuxTask: lagged {} messages", n);
                    }
                }
            }
            _ = cancel.cancelled() => {
                log::info!("MuxTask: cancelled");
                break;
            }
        }
    }
}

/// Raw frame sink 任务：直接发送解码帧到 sink
async fn run_raw_frame_sink_task(
    sink: Arc<RawSinkSource>,
    mut rx: broadcast::Receiver<DecodedFrame>,
    cancel: CancellationToken,
) {
    log::info!("RawFrameSinkTask: starting");

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(frame) => {
                        let _ = sink.writer.try_send(frame.data);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        log::info!("RawFrameSinkTask: channel closed");
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("RawFrameSinkTask: lagged {} messages", n);
                    }
                }
            }
            _ = cancel.cancelled() => {
                log::info!("RawFrameSinkTask: cancelled");
                break;
            }
        }
    }
}

fn dest_name(dest: &OutputDest) -> String {
    match dest {
        OutputDest::Network { url, .. } => url.clone(),
        OutputDest::RawFrame { .. } => "RawFrame".to_string(),
        OutputDest::RawPacket { .. } => "RawPacket".to_string(),
    }
}

// ============================================================================
// Builder API（便捷构建）
// ============================================================================

impl PipeV2Config {
    pub fn builder() -> PipeV2ConfigBuilder {
        PipeV2ConfigBuilder::default()
    }
}

#[derive(Default)]
pub struct PipeV2ConfigBuilder {
    input: Option<InputConfig>,
    outputs: Vec<OutputConfig>,
}

impl PipeV2ConfigBuilder {
    /// 设置网络输入源
    pub fn input_url(mut self, url: impl Into<String>) -> Self {
        self.input = Some(InputConfig::Network { url: url.into() });
        self
    }

    /// 添加 RTSP 输出（重编码）
    pub fn add_rtsp_output(mut self, url: impl Into<String>, encode: EncodeConfig) -> Self {
        self.outputs.push(OutputConfig {
            dest: OutputDest::Network {
                url: url.into(),
                format: "rtsp".to_string(),
            },
            encode: Some(encode),
        });
        self
    }

    /// 添加直接 remux 输出（不重编码）
    pub fn add_remux_output(mut self, url: impl Into<String>, format: impl Into<String>) -> Self {
        self.outputs.push(OutputConfig {
            dest: OutputDest::Network {
                url: url.into(),
                format: format.into(),
            },
            encode: None,
        });
        self
    }

    /// 添加原始帧输出
    pub fn add_raw_frame_output(mut self, sink: Arc<RawSinkSource>) -> Self {
        self.outputs.push(OutputConfig {
            dest: OutputDest::RawFrame { sink },
            encode: None,
        });
        self
    }

    /// 添加编码后包输出
    pub fn add_raw_packet_output(mut self, sink: Arc<RawSinkSource>, encode: EncodeConfig) -> Self {
        self.outputs.push(OutputConfig {
            dest: OutputDest::RawPacket { sink },
            encode: Some(encode),
        });
        self
    }

    pub fn build(self) -> PipeV2Config {
        PipeV2Config {
            input: self.input.expect("input is required"),
            outputs: self.outputs,
        }
    }
}
