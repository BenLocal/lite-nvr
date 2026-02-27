use std::{collections::HashMap, pin::Pin};

use futures::Stream;

use crate::{packet::RawPacket, stream::AvStream};
use bytes::Bytes;
use ffmpeg_next::{
    Dictionary, Rational,
    ffi::{
        AV_OPT_SEARCH_CHILDREN, AVIOContext, av_free, av_malloc, av_opt_set,
        avformat_alloc_output_context2, avio_alloc_context, avio_flush,
    },
    format::context::Output,
    media::Type as MediaType,
};
use std::ffi::CString;

pub struct AvOutput {
    inner: Output,
    /// input stream index -> AvStream (for time_base etc.)
    output_streams: HashMap<usize, AvStream>,
    /// input stream index -> output stream index (in inner)
    output_stream_index: HashMap<usize, usize>,
    interleaved: bool,
    have_written_header: bool,
    have_written_trailer: bool,
    /// output stream index -> last DTS written (enforce monotonically increasing DTS)
    last_dts: HashMap<usize, i64>,
}

/// Allocate RTSP output context without opening AVIO. The RTSP muxer will open
/// the URL when write_header() is called (FFmpeg design: do not call avio_open for RTSP).
fn output_rtsp_alloc_only(url: &str) -> anyhow::Result<Output> {
    unsafe {
        let mut output_ptr = std::ptr::null_mut();
        let url_c = CString::new(url).map_err(|e| anyhow::anyhow!("url CString: {}", e))?;
        let format = CString::new("rtsp").unwrap();
        match avformat_alloc_output_context2(
            &mut output_ptr,
            std::ptr::null_mut(),
            format.as_ptr(),
            url_c.as_ptr(),
        ) {
            0 => Ok(Output::wrap(output_ptr)),
            e => Err(anyhow::anyhow!(
                "avformat_alloc_output_context2(rtsp, url={:?}): {}",
                url,
                e
            )),
        }
    }
}

impl AvOutput {
    pub fn new(
        url: &str,
        format: Option<&str>,
        options: Option<Dictionary>,
    ) -> anyhow::Result<Self> {
        let output = match (format, options) {
            // RTSP: do not call avio_open; muxer opens URL in write_header().
            (Some("rtsp"), _) => output_rtsp_alloc_only(url)
                .map_err(|e| anyhow::anyhow!("output_rtsp_alloc_only(url={:?}): {}", url, e))?,
            (Some(fmt), Some(opts)) => ffmpeg_next::format::output_as_with(url, fmt, opts)
                .map_err(|e| {
                    anyhow::anyhow!("output_as_with(url={:?}, format={:?}): {:?}", url, fmt, e)
                })?,
            (Some(fmt), None) => ffmpeg_next::format::output_as(url, fmt).map_err(|e| {
                anyhow::anyhow!("output_as(url={:?}, format={:?}): {:?}", url, fmt, e)
            })?,
            (None, _) => ffmpeg_next::format::output(url)
                .map_err(|e| anyhow::anyhow!("output(url={:?}): {:?}", url, e))?,
        };
        Ok(Self {
            inner: output,
            output_streams: HashMap::new(),
            output_stream_index: HashMap::new(),
            interleaved: false,
            have_written_header: false,
            have_written_trailer: false,
            last_dts: HashMap::new(),
        })
    }

    pub fn add_stream(&mut self, stream: &AvStream) -> anyhow::Result<()> {
        let codec_parameters = stream.parameters();
        let codec_id = codec_parameters.id();
        let encoder = ffmpeg_next::encoder::find(codec_id)
            .ok_or_else(|| anyhow::anyhow!("encoder not found for codec_id {:?}", codec_id))?;
        let mut writer_stream = self
            .inner
            .add_stream(encoder)
            .map_err(|e| anyhow::anyhow!("add_stream(codec_id={:?}): {:?}", codec_id, e))?;
        writer_stream.set_parameters(codec_parameters.clone());
        let out_idx = writer_stream.index();
        self.output_stream_index.insert(stream.index(), out_idx);
        self.output_streams.insert(stream.index(), stream.clone());
        Ok(())
    }

    fn stream_time_base(&mut self, stream_index: usize) -> Rational {
        self.output_streams.get(&stream_index).unwrap().time_base()
    }

    /// Write a packet. `input_stream_index` is the input stream index (packet.stream() from input).
    pub fn write_packet(
        &mut self,
        input_stream_index: usize,
        mut packet: RawPacket,
    ) -> anyhow::Result<()> {
        let out_idx = match self.output_stream_index.get(&input_stream_index) {
            Some(&i) => i,
            None => return Err(anyhow::anyhow!("stream not found: {}", input_stream_index)),
        };
        if !self.have_written_header {
            self.inner.write_header()?;
            self.have_written_header = true;
        }
        let time_base = packet.time_base();

        let p = packet.get_mut();
        // Ensure PTS/DTS are set (FFmpeg deprecates unset timestamps)
        let pts = p.pts();
        let dts = p.dts();
        match (pts, dts) {
            (None, None) => {
                p.set_pts(Some(0));
                p.set_dts(Some(0));
            }
            (None, Some(d)) => {
                p.set_pts(Some(d));
            }
            (Some(_), None) => {
                p.set_dts(p.pts());
            }
            (Some(_), Some(_)) => {}
        }

        p.set_stream(out_idx);
        p.set_position(-1);
        let out_time_base = self.inner.stream(out_idx).unwrap().time_base();
        p.rescale_ts(time_base, out_time_base);

        // Enforce monotonically increasing DTS (muxer requirement)
        let dts = p.dts().unwrap_or(0);
        let last = self.last_dts.get(&out_idx).copied();
        let new_dts = match last {
            Some(last) if dts <= last => last + 1,
            _ => dts,
        };
        if new_dts != dts {
            p.set_dts(Some(new_dts));
            if p.pts().map(|x| x < new_dts).unwrap_or(true) {
                p.set_pts(Some(new_dts));
            }
        }
        self.last_dts.insert(out_idx, new_dts);

        if self.interleaved {
            p.write_interleaved(&mut self.inner)?;
        } else {
            p.write(&mut self.inner)?;
        }
        Ok(())
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        if self.have_written_header && !self.have_written_trailer {
            self.have_written_trailer = true;
            self.inner.write_trailer()?;
        }
        Ok(())
    }
}

/// Bounded capacity for mux output (writer→reader). Each message can be up to 256KB for H.264.
/// Large enough to avoid dropping under normal load (dropped packets break ffplay); still caps memory.
const MUX_OUTPUT_CHAN_CAP: usize = 256;

pub struct PacketContext {
    buffer: PacketBufferType,
    current_pts: Option<i64>,
    current_dts: Option<i64>,
    /// Video only: key frame flag
    pub current_is_key: bool,
    /// Video only: codec id (e.g. H264)
    pub current_codec_id: i32,
    /// Video only: width
    pub current_width: u32,
    /// Video only: height
    pub current_height: u32,
}

pub struct AvOutputStream {
    inner: Output,
    have_written_header: bool,
    have_written_trailer: bool,
    context: Box<PacketContext>,
    receiver: tokio::sync::mpsc::Receiver<OutputMessage>,
    /// Input stream index we're muxing (only one stream supported for now).
    input_stream_index: Option<usize>,
}

pub type PacketBufferType = tokio::sync::mpsc::Sender<OutputMessage>;

pub struct OutputMessage {
    pub data: Bytes,
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    /// Video only
    pub is_key: bool,
    pub codec_id: i32,
    pub width: u32,
    pub height: u32,
}

/// Writer half of a split `AvOutputStream`. Used to write packets from a separate task.
pub struct AvOutputStreamWriter {
    inner: Output,
    have_written_header: bool,
    have_written_trailer: bool,
    context: Box<PacketContext>,
    /// Input stream index we're muxing (only write packets with this stream index).
    input_stream_index: Option<usize>,
    /// Last DTS written (enforce monotonically increasing DTS for muxer).
    last_dts: Option<i64>,
}

impl AvOutputStreamWriter {
    pub fn write_packet(&mut self, mut packet: RawPacket) -> anyhow::Result<()> {
        let input_stream_index = match self.input_stream_index {
            Some(idx) => idx,
            None => return Err(anyhow::anyhow!("no stream added to output")),
        };
        if packet.index() != input_stream_index {
            return Ok(());
        }

        if !self.have_written_header {
            self.inner.write_header()?;
            self.have_written_header = true;
        }

        let time_base = packet.time_base();
        let p = packet.get_mut();
        p.set_stream(0);
        p.set_position(-1);
        let out_time_base = self.inner.stream(0).unwrap().time_base();
        p.rescale_ts(time_base, out_time_base);

        // Enforce monotonically increasing DTS (muxer requirement)
        let dts = p.dts().unwrap_or(0);
        let new_dts = match self.last_dts {
            Some(last) if dts <= last => last + 1,
            _ => dts,
        };
        if new_dts != dts {
            p.set_dts(Some(new_dts));
            if p.pts().map(|x| x < new_dts).unwrap_or(true) {
                p.set_pts(Some(new_dts));
            }
        }
        self.last_dts = Some(new_dts);

        self.context.current_pts = p.pts();
        self.context.current_dts = p.dts();
        self.context.current_is_key = p.is_key();
        if let Some(stream) = self.inner.stream(0) {
            let params = stream.parameters();
            if params.medium() == MediaType::Video {
                self.context.current_codec_id = params.id() as i32;
                let (w, h) = video_size_from_parameters(&params);
                self.context.current_width = w;
                self.context.current_height = h;
            }
        }

        log::debug!("write_packet: pts={:?}, dts={:?}", p.pts(), p.dts());
        log::debug!(
            "write_packet: time_base={:?}, out_time_base={:?}",
            time_base,
            out_time_base
        );
        p.write(&mut self.inner)?;

        self.context.current_pts = None;
        self.context.current_dts = None;
        self.context.current_is_key = false;
        self.context.current_codec_id = 0;
        self.context.current_width = 0;
        self.context.current_height = 0;

        Ok(())
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        if self.have_written_header && !self.have_written_trailer {
            self.have_written_trailer = true;
            self.inner.write_trailer()?;
        }
        Ok(())
    }
}

impl Drop for AvOutputStreamWriter {
    fn drop(&mut self) {
        let _ = self.finish();
        output_raw_packetized_buf_end(&mut self.inner);
    }
}

/// Reader half of a split `AvOutputStream`. Implements `Stream` and yields encoded packets.
pub struct AvOutputStreamReader {
    receiver: tokio::sync::mpsc::Receiver<OutputMessage>,
}

impl Stream for AvOutputStreamReader {
    type Item = OutputMessage;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.receiver.poll_recv(cx)
    }
}

impl AvOutputStream {
    /// Default buffer size for formats like mp4. Small chunks are fine for container output.
    const PACKET_SIZE: usize = 1024;
    /// Larger buffer for raw H.264 so the muxer does not split one NAL across multiple
    /// callbacks (which would produce invalid NALUs for consumers like ZLMediaKit).
    const PACKET_SIZE_H264: usize = 256 * 1024;

    pub fn new(format: &str) -> anyhow::Result<Self> {
        let mut inner = output_raw(format)?;
        if format == "mp4" {
            set_mp4_movflags(&mut inner)?;
        }
        let (sender, receiver) = tokio::sync::mpsc::channel(MUX_OUTPUT_CHAN_CAP);
        let mut context = Box::new(PacketContext {
            buffer: sender,
            current_pts: None,
            current_dts: None,
            current_is_key: false,
            current_codec_id: 0,
            current_width: 0,
            current_height: 0,
        });

        let buf_size = if format == "h264" {
            Self::PACKET_SIZE_H264
        } else {
            Self::PACKET_SIZE
        };

        // Initialize the custom IO context
        output_raw_packetized_buf_start(&mut inner, &mut context, buf_size);

        Ok(Self {
            inner,
            have_written_header: false,
            have_written_trailer: false,
            context,
            receiver,
            input_stream_index: None,
        })
    }

    /// Add one output stream (e.g. video). Must be called before writing. Only one stream is supported.
    pub fn add_stream(&mut self, stream: &AvStream) -> anyhow::Result<()> {
        let codec_parameters = stream.parameters();
        let mut writer_stream = self
            .inner
            .add_stream(ffmpeg_next::encoder::find(codec_parameters.id()))?;
        writer_stream.set_parameters(codec_parameters.clone());
        self.input_stream_index = Some(stream.index());
        Ok(())
    }

    /// Split into writer (for `write_packet` in another task) and reader (for consuming as `Stream`).
    pub fn into_split(self) -> (AvOutputStreamWriter, AvOutputStreamReader) {
        let this = std::mem::ManuallyDrop::new(self);
        unsafe {
            let inner = std::ptr::read(&this.inner);
            let have_written_header = this.have_written_header;
            let have_written_trailer = this.have_written_trailer;
            let context = std::ptr::read(&this.context);
            let receiver = std::ptr::read(&this.receiver);
            let input_stream_index = this.input_stream_index;
            (
                AvOutputStreamWriter {
                    inner,
                    have_written_header,
                    have_written_trailer,
                    context,
                    input_stream_index,
                    last_dts: None,
                },
                AvOutputStreamReader { receiver },
            )
        }
    }
}

/// Reads video width/height from codec parameters (not exposed by ffmpeg-next).
fn video_size_from_parameters(params: &ffmpeg_next::codec::Parameters) -> (u32, u32) {
    unsafe {
        let ptr = params.as_ptr() as *const ffmpeg_next::ffi::AVCodecParameters;
        let w = (*ptr).width;
        let h = (*ptr).height;
        (w.max(0) as u32, h.max(0) as u32)
    }
}

/// Set movflags for MP4 so the muxer works with non-seekable output (e.g. our custom IO).
/// Without this, the muxer would need to seek to write moov and would produce an invalid file.
fn set_mp4_movflags(output: &mut Output) -> anyhow::Result<()> {
    unsafe {
        let name = CString::new("movflags").unwrap();
        let value = CString::new("frag_keyframe+empty_moov").unwrap();
        let ret = av_opt_set(
            output.as_mut_ptr() as *mut std::ffi::c_void,
            name.as_ptr(),
            value.as_ptr(),
            AV_OPT_SEARCH_CHILDREN,
        );
        if ret != 0 {
            return Err(anyhow::anyhow!("av_opt_set movflags failed: {}", ret));
        }
    }
    Ok(())
}

/// ------------------------------------------------------------
/// Helper functions for raw output
/// Copy from https://github.com/oddity-ai/video-rs/blob/main/src/ffi.rs
/// ------------------------------------------------------------

/// This function is similar to the existing bindings in ffmpeg-next like `output` and `output_as`,
/// but does not assume that it is opening a file-like context. Instead, it opens a raw output,
/// without a file attached.
///
/// Combined with the `output_raw_buf_start` and `output_raw_buf_end` functions, this can be used to
/// write to a buffer instead of a file.
///
/// # Arguments
///
/// * `format` - String to indicate the container format, like "mp4".
///
/// # Example
///
/// ```ignore
/// let output = ffi::output_raw("mp4");
///
/// output_raw_buf_start(&mut output);
/// output.write_header()?;
/// let buf output_raw_buf_end(&mut output);
/// println!("{}", buf.len());
/// ```
fn output_raw(format: &str) -> anyhow::Result<Output> {
    unsafe {
        let mut output_ptr = std::ptr::null_mut();
        let format = std::ffi::CString::new(format).unwrap();
        match avformat_alloc_output_context2(
            &mut output_ptr,
            std::ptr::null_mut(),
            format.as_ptr(),
            std::ptr::null(),
        ) {
            0 => Ok(Output::wrap(output_ptr)),
            e => Err(anyhow::anyhow!("failed to alloc output context: {}", e)),
        }
    }
}

/// This function initializes an IO context for the `Output` that packetizes individual writes. Each
/// write is pushed onto a packet buffer (a collection of buffers, each being a packet).
///
/// The callee must invoke `output_raw_packetized_buf_end` soon after calling this function. The
/// `Vec` pointed to by `packet_buffer` must live between invocation of this function and
/// `output_raw_packetized_buf_end`!
///
/// Not calling `output_raw_packetized_buf_end` after calling this function will result in memory
/// leaking.
///
/// # Arguments
///
/// * `output` - Output context to start write on.
/// * `packet_buffer` - Packet buffer to push buffers onto. Must live until
///   `output_raw_packetized_buf`.
/// * `max_packet_size` - Maximum size per packet.
pub fn output_raw_packetized_buf_start(
    output: &mut Output,
    packet_context: &mut Box<PacketContext>,
    max_packet_size: usize,
) {
    unsafe {
        let buffer = av_malloc(max_packet_size) as *mut u8;

        // Create a custom IO context around our buffer.
        let io: *mut AVIOContext = avio_alloc_context(
            buffer,
            max_packet_size.try_into().unwrap(),
            // Set stream to WRITE.
            1,
            // Pass on a pointer *UNSAFE* to the packet context, assuming the packet context will live
            // long enough.
            packet_context.as_mut() as *mut PacketContext as *mut std::ffi::c_void,
            // No `read_packet`.
            None,
            // Passthrough for `write_packet`.
            // XXX: Doing a manual transmute here to match the expected callback function
            // signature. Since it changed since ffmpeg 7 and we don't know during compile time
            // what verion we're dealing with, this trick will convert to the either the signature
            // where the buffer argument is `*const u8` or `*mut u8`.
            #[allow(clippy::missing_transmute_annotations)]
            Some(std::mem::transmute::<*const (), _>(
                output_raw_buf_start_callback as _,
            )),
            // No `seek`.
            None,
        );

        // Setting `max_packet_size` will let the underlying IO stream know that this buffer must be
        // treated as packetized.
        (*io).max_packet_size = max_packet_size.try_into().unwrap();

        // Assign IO to output context.
        (*output.as_mut_ptr()).pb = io;
    }
}

/// This function cleans up the IO context used for packetized writing created by
/// `output_raw_packetized_buf_start`.
///
/// # Arguments
///
/// * `output` - Output context to end write on.
pub fn output_raw_packetized_buf_end(output: &mut Output) {
    unsafe {
        let output_pb = (*output.as_mut_ptr()).pb;

        // One last flush (might incur write, most likely won't).
        avio_flush(output_pb);

        // Note: No need for handling `opaque` as it is managed by Rust code anyway and will be
        // freed by it.

        // We do need to free the buffer itself though (we allocatd it manually earlier).
        av_free((*output_pb).buffer as *mut std::ffi::c_void);
        // And deallocate the entire IO context.
        av_free(output_pb as *mut std::ffi::c_void);

        // Reset the `pb` field or `avformat_close` will try to free it!
        ((*output.as_mut_ptr()).pb) = std::ptr::null_mut::<AVIOContext>();
    }
}

/// Passthrough function that is passed to `libavformat` in `avio_alloc_context` and pushes buffers
/// from a packetized stream onto the packet buffer held in `opaque`.
extern "C" fn output_raw_buf_start_callback(
    opaque: *mut std::ffi::c_void,
    buffer: *const u8,
    buffer_size: i32,
) -> i32 {
    unsafe {
        // Acquire a reference to the packet context transmuted from the `opaque` gotten through
        // `libavformat`.
        let packet_context: &mut PacketContext = &mut *(opaque as *mut PacketContext);
        // Push the current packet onto the packet buffer with PTS/DTS.
        let buf = std::slice::from_raw_parts(buffer, buffer_size as usize);
        let data = Bytes::copy_from_slice(buf);
        let msg = OutputMessage {
            data,
            pts: packet_context.current_pts,
            dts: packet_context.current_dts,
            is_key: packet_context.current_is_key,
            codec_id: packet_context.current_codec_id,
            width: packet_context.current_width,
            height: packet_context.current_height,
        };
        if packet_context.buffer.try_send(msg).is_err() {
            log::warn!(
                "mux output channel full, dropping packet ({} bytes)",
                buffer_size
            );
        }
    }

    // Number of bytes written.
    buffer_size
}
