use std::fmt::{Display, Formatter};
use std::sync::Arc;

use bytes::Bytes;
use ffmpeg_next::Rational;

use crate::output::OutputMessage;
use crate::packet::RawPacket;

pub type RawFrameSender = tokio::sync::broadcast::Sender<RawFrameCmd>;
pub type RawFrameReceiver = tokio::sync::broadcast::Receiver<RawFrameCmd>;

#[derive(Clone)]
pub enum RawFrameCmd {
    Data(RawFrame),
    EOF,
}

#[derive(Clone)]
pub enum RawFrame {
    Video(RawVideoFrame),
    Audio(RawAudioFrame),
}

#[derive(Clone)]
pub struct RawAudioFrame {
    frame: Arc<ffmpeg_next::frame::Audio>,
}

impl RawAudioFrame {
    pub fn pts(&self) -> Option<i64> {
        self.frame.pts()
    }

    pub fn timestamp(&self) -> Option<i64> {
        self.frame.timestamp()
    }

    pub fn format(&self) -> ffmpeg_next::format::Sample {
        self.frame.format()
    }

    pub fn get_mut(&mut self) -> &mut ffmpeg_next::frame::Audio {
        Arc::make_mut(&mut self.frame)
    }

    pub fn as_audio(&self) -> &ffmpeg_next::frame::Audio {
        &self.frame
    }
}

impl From<ffmpeg_next::frame::Audio> for RawAudioFrame {
    fn from(frame: ffmpeg_next::frame::Audio) -> Self {
        Self {
            frame: Arc::new(frame),
        }
    }
}

#[derive(Clone)]
pub struct RawVideoFrame {
    frame: Arc<ffmpeg_next::frame::Video>,
}

impl From<ffmpeg_next::frame::Video> for RawVideoFrame {
    fn from(frame: ffmpeg_next::frame::Video) -> Self {
        Self {
            frame: Arc::new(frame),
        }
    }
}

/// Converts a raw video packet into a RawFrame::Video. Used when input is already raw (e.g. RAWVIDEO)
/// and needs to be fed to the encoder as frame. Requires stream dimensions and pixel format.
pub fn packet_to_raw_video_frame(
    packet: RawPacket,
    width: u32,
    height: u32,
    pixel_format: ffmpeg_next::format::Pixel,
) -> anyhow::Result<RawFrame> {
    use ffmpeg_next::frame::Video;
    if width == 0 || height == 0 {
        anyhow::bail!("invalid video size {}x{}", width, height);
    }
    if pixel_format == ffmpeg_next::format::Pixel::None {
        anyhow::bail!("invalid pixel format for raw video");
    }
    let mut frame = Video::new(pixel_format, width, height);
    let packet_data = packet.data();
    let frame_buf = frame.data_mut(0);
    let copy_len = packet_data.len().min(frame_buf.len());
    frame_buf[..copy_len].copy_from_slice(&packet_data[..copy_len]);
    frame.set_pts(packet.pts());
    Ok(RawFrame::Video(RawVideoFrame::from(frame)))
}

impl RawVideoFrame {
    pub fn width(&self) -> u32 {
        self.frame.width()
    }

    pub fn height(&self) -> u32 {
        self.frame.height()
    }

    pub fn format(&self) -> ffmpeg_next::format::Pixel {
        self.frame.format()
    }

    pub fn pts(&self) -> Option<i64> {
        self.frame.pts()
    }

    pub fn get_mut(&mut self) -> &mut ffmpeg_next::frame::Video {
        Arc::make_mut(&mut self.frame)
    }

    /// Mutable access for frame *properties* (pts, picture type, ...) only.
    /// When the frame is shared (a decoded frame fanned out to several
    /// subscribers), this makes a new `AVFrame` referencing the same
    /// ref-counted pixel buffers instead of copying them like [`Self::get_mut`].
    /// Never write pixel data through it: the buffers are still shared.
    pub fn props_mut(&mut self) -> &mut ffmpeg_next::frame::Video {
        if Arc::get_mut(&mut self.frame).is_none() {
            let mut shallow = ffmpeg_next::frame::Video::empty();
            // SAFETY: both pointers are valid AVFrames owned by live wrappers;
            // av_frame_ref only adds references to the source's buffers and
            // copies its properties into the empty destination.
            let ret = unsafe {
                ffmpeg_next::ffi::av_frame_ref(shallow.as_mut_ptr(), self.frame.as_ptr())
            };
            if ret < 0 {
                // Not ref-counted (cannot share buffers): fall back to a copy.
                return Arc::make_mut(&mut self.frame);
            }
            self.frame = Arc::new(shallow);
        }
        // Unique now, so make_mut hands out the frame without copying.
        Arc::make_mut(&mut self.frame)
    }

    pub fn data(&self) -> Bytes {
        Bytes::copy_from_slice(self.frame.data(0))
    }

    /// Every plane, tightly packed (no row padding), in the layout
    /// `av_image_get_buffer_size(format, width, height, 1)` describes: e.g.
    /// yuv420p is the full Y plane, then U, then V. [`Self::data`] only has
    /// plane 0, with its stride padding.
    pub fn packed_data(&self) -> anyhow::Result<Bytes> {
        let f = &*self.frame;
        let fmt: ffmpeg_next::ffi::AVPixelFormat = f.format().into();
        let (w, h) = (f.width() as i32, f.height() as i32);
        // SAFETY: plain size computation from format and dimensions.
        let size = unsafe { ffmpeg_next::ffi::av_image_get_buffer_size(fmt, w, h, 1) };
        if size < 0 {
            anyhow::bail!("cannot pack {:?} {}x{} frame", f.format(), w, h);
        }
        let mut out = vec![0u8; size as usize];
        // SAFETY: `out` holds exactly `size` bytes for this format/geometry,
        // and the source data/linesize arrays come from a valid, allocated
        // frame of that format and geometry.
        let ret = unsafe {
            let src = f.as_ptr();
            ffmpeg_next::ffi::av_image_copy_to_buffer(
                out.as_mut_ptr(),
                size,
                (*src).data.as_ptr() as *const *const u8,
                (*src).linesize.as_ptr(),
                fmt,
                w,
                h,
                1,
            )
        };
        if ret < 0 {
            anyhow::bail!("av_image_copy_to_buffer failed: {ret}");
        }
        Ok(Bytes::from(out))
    }

    /// Borrow the inner decoded frame (all planes) — needed to feed a scaler.
    /// `data()` only exposes plane 0.
    pub fn as_video(&self) -> &ffmpeg_next::frame::Video {
        &self.frame
    }

    pub fn is_key(&self) -> bool {
        self.frame.is_key()
    }

    pub fn pts_ms(&self, time_base: Rational) -> Option<u64> {
        self.frame.pts().map(|pts| {
            let pts_u = pts.max(0) as u64;
            let num = time_base.numerator() as u64;
            let den = time_base.denominator() as u64;
            pts_u * num / den
        })
    }
}

#[derive(Debug, Default)]
pub struct VideoFrame {
    pub data: Bytes,
    pub width: u32,
    pub height: u32,
    // AVPixelFormat
    pub format: i32,
    pub pts: i64,
    pub dts: i64,
    pub is_key: bool,
    // AVCodecID
    pub codec_id: i32,
}

impl VideoFrame {
    pub fn new(
        data: Vec<u8>,
        width: u32,
        height: u32,
        format: i32,
        pts: i64,
        dts: i64,
        is_key: bool,
        codec_id: i32,
    ) -> Self {
        Self {
            data: Bytes::from(data),
            width,
            height,
            format,
            pts,
            dts,
            is_key,
            codec_id,
        }
    }

    pub fn new_encoded(data: Vec<u8>, width: u32, height: u32, codec_id: i32) -> Self {
        Self {
            data: Bytes::from(data),
            width: width,
            height: height,
            codec_id: codec_id,
            ..Default::default()
        }
    }

    pub fn pts_ms(&self, time_base: Rational) -> f64 {
        let pts_u = self.pts.max(0) as f64;
        let num = time_base.numerator() as f64;
        let den = time_base.denominator() as f64;
        pts_u * num * 1000.0 / den
    }

    pub fn dts_ms(&self, time_base: Rational) -> f64 {
        let dts_u = self.dts.max(0) as f64;
        let num = time_base.numerator() as f64;
        let den = time_base.denominator() as f64;
        dts_u * num * 1000.0 / den
    }
}

impl Display for VideoFrame {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(
            f,
            "VideoFrame data_len: {}, width: {}, height: {}, format: {}, pts: {}, dts: {}, is_key: {}, codec_id: {}",
            self.data.len(),
            self.width,
            self.height,
            self.format,
            self.pts,
            self.dts,
            self.is_key,
            self.codec_id
        )
    }
}

impl Clone for VideoFrame {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            width: self.width,
            height: self.height,
            format: self.format,
            pts: self.pts,
            dts: self.dts,
            is_key: self.is_key,
            codec_id: self.codec_id,
        }
    }
}

impl TryFrom<RawFrame> for VideoFrame {
    type Error = anyhow::Error;
    fn try_from(value: RawFrame) -> Result<Self, Self::Error> {
        if let RawFrame::Video(frame) = value {
            Ok(Self {
                data: frame.packed_data()?,
                width: frame.width(),
                height: frame.height(),
                format: frame.format() as i32,
                pts: frame.pts().unwrap_or(0),
                dts: 0,
                is_key: frame.is_key(),
                codec_id: ffmpeg_next::codec::Id::None as i32,
            })
        } else {
            Err(anyhow::anyhow!("not a video frame"))
        }
    }
}

impl From<OutputMessage> for VideoFrame {
    fn from(value: OutputMessage) -> Self {
        Self {
            data: value.data,
            width: value.width,
            height: value.height,
            format: 0,
            pts: value.pts.unwrap_or(0),
            dts: value.dts.unwrap_or(0),
            is_key: value.is_key,
            codec_id: value.codec_id,
        }
    }
}

impl From<RawPacket> for VideoFrame {
    fn from(packet: RawPacket) -> Self {
        Self {
            data: packet.data(),
            width: 0,
            height: 0,
            format: 0,
            pts: packet.pts().unwrap_or(0),
            dts: packet.dts().unwrap_or(0),
            is_key: packet.is_key(),
            codec_id: 0,
        }
    }
}

#[cfg(test)]
#[path = "frame_test.rs"]
mod frame_test;
