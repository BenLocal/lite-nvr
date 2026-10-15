//! Rockchip-only V4L2 capture that exports every capture buffer as a DMABUF,
//! so RKMPP encodes straight from the capture memory with no CPU copy (see
//! `docs/v4l2-dmabuf-rkmpp-spec.md`). Packets still point into the mmap'd
//! buffers, so CPU consumers (snapshots, detection) keep working.
//!
//! Opt-in with `NVR_V4L2_DMABUF=1`; anything unsupported falls back to the
//! FFmpeg V4L2 input.

use std::os::fd::{AsFd, AsRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use ffmpeg_next::Rational;
use rustix::io::Errno;

use crate::input::ReadOutcome;
use crate::packet::RawPacket;
use crate::stream::AvStream;

/// Drivers whose capture buffers were verified to export and import into
/// RKMPP (Phase 0, 2026-10-15).
const SUPPORTED_DRIVERS: &[&str] = &["rk_hdmirx", "rkcif"];
/// Buffers requested from the driver: enough for the encoder queue (8) plus
/// frames held by RKMPP and in transit.
const BUFFER_COUNT: u32 = 24;
/// Frames are dropped once this few buffers would be left with the driver.
const DRIVER_RESERVE: usize = 4;
/// How long one read waits for a frame, bounding cancellation latency.
const READ_WAIT: Duration = Duration::from_millis(100);

/// Whether the DMABUF capture may be used (`NVR_V4L2_DMABUF=1`).
pub(crate) fn enabled() -> bool {
    enabled_from(std::env::var("NVR_V4L2_DMABUF").ok().as_deref())
}

pub(crate) fn enabled_from(value: Option<&str>) -> bool {
    value.map(str::trim) == Some("1")
}

/// Geometry of one captured picture (single plane, tightly described by the
/// driver's bytes-per-line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub pixel: ffmpeg_next::format::Pixel,
    pub bytes_per_line: u32,
    pub size: u32,
}

/// A captured buffer's DMABUF, kept open (and the buffer away from the
/// driver) while any packet, frame or encoder still references it.
#[derive(Clone)]
pub struct DmabufRef {
    fd: RawFd,
    layout: Layout,
    _keep: Arc<dyn Send + Sync>,
}

impl DmabufRef {
    pub(crate) fn new(fd: RawFd, layout: Layout, keep: Arc<dyn Send + Sync>) -> Self {
        Self {
            fd,
            layout,
            _keep: keep,
        }
    }

    pub fn fd(&self) -> RawFd {
        self.fd
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }
}

/// DRM descriptor for handing a captured buffer to RKMPP: one linear object,
/// one layer; NV12's chroma plane follows the luma plane. RKMPP takes the
/// pixel format from the encoder's `sw_pix_fmt`, not from the layer format.
pub(crate) fn drm_descriptor(fd: RawFd, layout: &Layout) -> ffmpeg_next::ffi::AVDRMFrameDescriptor {
    // SAFETY: AVDRMFrameDescriptor is plain old data; all-zero is valid.
    let mut desc: ffmpeg_next::ffi::AVDRMFrameDescriptor = unsafe { std::mem::zeroed() };
    desc.nb_objects = 1;
    desc.objects[0].fd = fd;
    desc.objects[0].size = layout.size as usize;
    desc.objects[0].format_modifier = 0; // DRM_FORMAT_MOD_LINEAR
    desc.nb_layers = 1;
    let layer = &mut desc.layers[0];
    layer.format = drm_fourcc(layout.pixel);
    layer.planes[0].pitch = layout.bytes_per_line as isize;
    if layout.pixel == ffmpeg_next::format::Pixel::NV12 {
        layer.nb_planes = 2;
        layer.planes[1].offset = layout.bytes_per_line as isize * layout.height as isize;
        layer.planes[1].pitch = layout.bytes_per_line as isize;
    } else {
        layer.nb_planes = 1;
    }
    desc
}

/// Descriptor plus the capture buffer it describes, owned by the DRM frame.
struct DrmHold {
    desc: ffmpeg_next::ffi::AVDRMFrameDescriptor,
    _dmabuf: DmabufRef,
}

unsafe extern "C" fn release_drm(opaque: *mut std::ffi::c_void, _data: *mut u8) {
    // SAFETY: created by Box::into_raw in `drm_frame`; FFmpeg frees once.
    drop(unsafe { Box::from_raw(opaque as *mut DrmHold) });
}

/// A DRM_PRIME frame for `dmabuf`: the encoder imports the buffer by fd and
/// keeps it (and so the capture buffer) referenced until it is encoded.
pub(crate) fn drm_frame(
    dmabuf: &DmabufRef,
    pts: Option<i64>,
) -> anyhow::Result<ffmpeg_next::frame::Video> {
    let layout = dmabuf.layout();
    let hold = Box::into_raw(Box::new(DrmHold {
        desc: drm_descriptor(dmabuf.fd(), &layout),
        _dmabuf: dmabuf.clone(),
    }));
    let mut frame = ffmpeg_next::frame::Video::empty();
    // SAFETY: `hold` stays alive until `release_drm` runs when the frame's
    // last buffer reference goes; data[0] points at its descriptor, as
    // AV_PIX_FMT_DRM_PRIME requires.
    unsafe {
        let buffer = ffmpeg_next::ffi::av_buffer_create(
            std::ptr::addr_of_mut!((*hold).desc) as *mut u8,
            std::mem::size_of::<ffmpeg_next::ffi::AVDRMFrameDescriptor>(),
            Some(release_drm),
            hold as *mut std::ffi::c_void,
            0,
        );
        if buffer.is_null() {
            drop(Box::from_raw(hold));
            anyhow::bail!("out of memory for a DRM frame");
        }
        let raw = frame.as_mut_ptr();
        (*raw).buf[0] = buffer;
        (*raw).data[0] = std::ptr::addr_of_mut!((*hold).desc) as *mut u8;
        (*raw).format = ffmpeg_next::ffi::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as i32;
        (*raw).width = i32::try_from(layout.width)?;
        (*raw).height = i32::try_from(layout.height)?;
    }
    frame.set_pts(pts);
    Ok(frame)
}

fn drm_fourcc(pixel: ffmpeg_next::format::Pixel) -> u32 {
    let code = |s: &[u8; 4]| u32::from_le_bytes(*s);
    match pixel {
        // V4L2 BGR3 stores B,G,R bytes: DRM_FORMAT_RGB888 ([23:0] R:G:B little endian).
        ffmpeg_next::format::Pixel::BGR24 => code(b"RG24"),
        ffmpeg_next::format::Pixel::NV12 => code(b"NV12"),
        _ => 0,
    }
}

/// Frame rate from DV timings: pixel clock over the full frame size.
pub(crate) fn frame_rate_from_timings(
    pixelclock: u64,
    total_width: u64,
    total_height: u64,
) -> Option<Rational> {
    let frame = total_width.checked_mul(total_height)?;
    if pixelclock == 0 || frame == 0 {
        return None;
    }
    let rate = ((pixelclock as f64 / frame as f64) * 1000.0).round() as i32;
    (rate > 0).then(|| Rational(rate, 1000).reduce())
}

struct Buffer {
    map: *mut std::ffi::c_void,
    len: usize,
    dmabuf: OwnedFd,
}

struct Device {
    fd: OwnedFd,
    buffers: Vec<Buffer>,
    in_flight: AtomicUsize,
    streaming: AtomicBool,
}

// SAFETY: the mapped pointers are only read (PROT_READ) and stay valid until
// Drop unmaps them, which happens after the last reference is gone.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

impl Device {
    fn queue(&self, index: u32) -> Result<(), Errno> {
        let mut planes = [sys::Plane::default(); 1];
        let mut buf = sys::Buffer::mmap(index, &mut planes);
        // SAFETY: `buf` and `planes` live across the call; QBUF only reads them.
        unsafe { rustix::ioctl::ioctl(&self.fd, sys::Qbuf::new(&mut buf)) }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        if self.streaming.swap(false, Ordering::SeqCst) {
            let _ = sys::stream(&self.fd, false);
        }
        for buffer in &self.buffers {
            // SAFETY: mapped in `DmabufCapture::open` with this length; nothing
            // references it any more (all packets hold the Arc<Device>).
            let _ = unsafe { rustix::mm::munmap(buffer.map, buffer.len) };
        }
    }
}

/// Per-packet release context: requeues its buffer when FFmpeg frees the
/// packet's (last) buffer reference.
struct Release {
    device: Arc<Device>,
    index: u32,
}

unsafe extern "C" fn release_buffer(opaque: *mut std::ffi::c_void, _data: *mut u8) {
    // SAFETY: `opaque` was created by Box::into_raw in `DmabufCapture::read`
    // and FFmpeg calls this exactly once.
    let release = unsafe { Box::from_raw(opaque as *mut Release) };
    if release.device.streaming.load(Ordering::SeqCst)
        && let Err(e) = release.device.queue(release.index)
    {
        log::warn!("v4l2 dmabuf: requeue buffer {}: {e}", release.index);
    }
    release.device.in_flight.fetch_sub(1, Ordering::SeqCst);
}

pub(crate) struct DmabufCapture {
    device: Arc<Device>,
    layout: Layout,
    stream: AvStream,
    /// Added to the driver's monotonic timestamps to get the wall-clock
    /// microseconds FFmpeg's V4L2 input produces, so timing stays the same.
    clock_offset_us: i64,
    dropped: Arc<AtomicU64>,
}

impl DmabufCapture {
    /// Open `path` for DMABUF capture, or explain why it cannot be used.
    pub(crate) fn open(path: &str, requested_rate: Option<Rational>) -> anyhow::Result<Self> {
        use rustix::fs::{Mode, OFlags};
        let fd = rustix::fs::open(
            path,
            OFlags::RDWR | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| anyhow::anyhow!("open {path}: {e}"))?;

        // SAFETY: QUERYCAP only fills the returned struct.
        let cap = unsafe { rustix::ioctl::ioctl(&fd, sys::Querycap::new()) }
            .map_err(|e| anyhow::anyhow!("not a V4L2 device ({e})"))?;
        let driver = sys::c_str(&cap.driver);
        anyhow::ensure!(
            SUPPORTED_DRIVERS.contains(&driver.as_str()),
            "driver {driver} not verified for DMABUF capture"
        );
        anyhow::ensure!(
            cap.device_caps & sys::CAP_VIDEO_CAPTURE_MPLANE != 0
                && cap.device_caps & sys::CAP_STREAMING != 0,
            "device lacks multiplanar streaming capture"
        );

        let mut format = sys::Format::mplane();
        // SAFETY: `format` is a valid v4l2_format for G_FMT to fill.
        unsafe { rustix::ioctl::ioctl(&fd, sys::GFmt::new(&mut format)) }
            .map_err(|e| anyhow::anyhow!("G_FMT: {e}"))?;
        let pix = format.pix_mp;
        anyhow::ensure!(
            pix.num_planes == 1,
            "{} planes not supported",
            pix.num_planes
        );
        let pixel = match &pix.pixelformat.to_le_bytes() {
            b"BGR3" => ffmpeg_next::format::Pixel::BGR24,
            b"NV12" => ffmpeg_next::format::Pixel::NV12,
            other => anyhow::bail!(
                "pixel format {} not supported",
                String::from_utf8_lossy(other)
            ),
        };
        let plane = pix.plane_fmt[0];
        let layout = Layout {
            width: pix.width,
            height: pix.height,
            pixel,
            bytes_per_line: plane.bytesperline,
            size: plane.sizeimage,
        };
        let minimum = u64::from(layout.bytes_per_line)
            * u64::from(layout.height)
            * if pixel == ffmpeg_next::format::Pixel::NV12 {
                3
            } else {
                2
            }
            / 2;
        anyhow::ensure!(
            layout.width > 0 && layout.height > 0 && u64::from(layout.size) >= minimum,
            "unexpected layout {layout:?}"
        );

        let mut request = sys::RequestBuffers {
            count: BUFFER_COUNT,
            type_: sys::BUF_TYPE_VIDEO_CAPTURE_MPLANE,
            memory: sys::MEMORY_MMAP,
            ..Default::default()
        };
        // SAFETY: `request` is a valid v4l2_requestbuffers.
        unsafe { rustix::ioctl::ioctl(&fd, sys::Reqbufs::new(&mut request)) }
            .map_err(|e| anyhow::anyhow!("REQBUFS: {e}"))?;
        anyhow::ensure!(
            request.count as usize > DRIVER_RESERVE + 1,
            "driver granted only {} buffers",
            request.count
        );

        let mut buffers = Vec::with_capacity(request.count as usize);
        for index in 0..request.count {
            let mut planes = [sys::Plane::default(); 1];
            let mut buf = sys::Buffer::mmap(index, &mut planes);
            // SAFETY: `buf` points at `planes`, both live across the call.
            unsafe { rustix::ioctl::ioctl(&fd, sys::Querybuf::new(&mut buf)) }
                .map_err(|e| anyhow::anyhow!("QUERYBUF {index}: {e}"))?;
            let len = planes[0].length as usize;
            anyhow::ensure!(len >= layout.size as usize, "buffer {index} too small");
            // SAFETY: maps the driver buffer read-only at the offset QUERYBUF
            // returned; unmapped in Device::drop (or below on failure).
            let map = unsafe {
                rustix::mm::mmap(
                    std::ptr::null_mut(),
                    len,
                    rustix::mm::ProtFlags::READ,
                    rustix::mm::MapFlags::SHARED,
                    &fd,
                    planes[0].m & 0xffff_ffff,
                )
            }
            .map_err(|e| anyhow::anyhow!("mmap {index}: {e}"))?;
            let mut export = sys::ExportBuffer {
                type_: sys::BUF_TYPE_VIDEO_CAPTURE_MPLANE,
                index,
                flags: (rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC).bits(),
                ..Default::default()
            };
            // SAFETY: `export` is a valid v4l2_exportbuffer.
            let exported = unsafe { rustix::ioctl::ioctl(&fd, sys::Expbuf::new(&mut export)) };
            if let Err(e) = exported {
                // SAFETY: just mapped above with this length.
                let _ = unsafe { rustix::mm::munmap(map, len) };
                anyhow::bail!("EXPBUF {index}: {e}");
            }
            // SAFETY: EXPBUF returned a new file descriptor that we now own.
            let dmabuf = unsafe { <OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(export.fd) };
            buffers.push(Buffer { map, len, dmabuf });
        }

        let device = Arc::new(Device {
            fd,
            buffers,
            in_flight: AtomicUsize::new(0),
            streaming: AtomicBool::new(false),
        });
        for index in 0..request.count {
            device
                .queue(index)
                .map_err(|e| anyhow::anyhow!("QBUF {index}: {e}"))?;
        }
        sys::stream(&device.fd, true).map_err(|e| anyhow::anyhow!("STREAMON: {e}"))?;
        device.streaming.store(true, Ordering::SeqCst);

        let rate = requested_rate
            .or_else(|| sys::dv_frame_rate(&device.fd))
            .unwrap_or_else(|| {
                log::warn!("v4l2 dmabuf: {path}: no frame rate known, assuming 60");
                Rational(60, 1)
            });
        let stream = AvStream::new(
            0,
            raw_video_parameters(&layout)?,
            Rational(1, 1_000_000),
            rate,
        );
        let clock_offset_us = sys::realtime_minus_monotonic_us();
        log::info!(
            "v4l2 dmabuf: {path} ({driver}) {}x{} {:?} bpl={} buffers={} rate={}/{}",
            layout.width,
            layout.height,
            layout.pixel,
            layout.bytes_per_line,
            request.count,
            rate.numerator(),
            rate.denominator()
        );
        Ok(Self {
            device,
            layout,
            stream,
            clock_offset_us,
            dropped: Arc::default(),
        })
    }

    pub(crate) fn stream(&self) -> &AvStream {
        &self.stream
    }

    /// Frames dropped because too few buffers were left with the driver.
    pub(crate) fn drops(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.dropped)
    }

    pub(crate) fn read(&mut self) -> ReadOutcome {
        let device = &self.device;
        let mut fds = [rustix::event::PollFd::new(
            &device.fd,
            rustix::event::PollFlags::IN,
        )];
        let wait = rustix::time::Timespec {
            tv_sec: 0,
            tv_nsec: READ_WAIT.as_nanos() as _,
        };
        match rustix::event::poll(&mut fds, Some(&wait)) {
            Ok(0) | Err(Errno::INTR) => return ReadOutcome::Again,
            Ok(_) => {}
            Err(e) => return ReadOutcome::Failed(errno(e)),
        }

        let mut planes = [sys::Plane::default(); 1];
        let mut buf = sys::Buffer::mmap(0, &mut planes);
        // SAFETY: `buf` points at `planes`; DQBUF fills both.
        if let Err(e) = unsafe { rustix::ioctl::ioctl(&device.fd, sys::Dqbuf::new(&mut buf)) } {
            return match e {
                Errno::AGAIN | Errno::INTR => ReadOutcome::Again,
                e => ReadOutcome::Failed(errno(e)),
            };
        }
        let index = buf.index;
        let Some(buffer) = device.buffers.get(index as usize) else {
            return ReadOutcome::Failed(ffmpeg_next::Error::Bug);
        };
        if buf.flags & sys::BUF_FLAG_ERROR != 0
            || device.in_flight.load(Ordering::SeqCst) + DRIVER_RESERVE >= device.buffers.len()
        {
            if buf.flags & sys::BUF_FLAG_ERROR == 0 {
                let dropped = self.dropped.fetch_add(1, Ordering::Relaxed) + 1;
                if dropped % 300 == 1 {
                    log::warn!("v4l2 dmabuf: buffers held downstream, dropped {dropped} frames");
                }
            }
            if let Err(e) = device.queue(index) {
                return ReadOutcome::Failed(errno(e));
            }
            return ReadOutcome::Again;
        }

        let bytes = (planes[0].bytesused as usize).min(buffer.len);
        let Ok(size) = i32::try_from(bytes) else {
            let _ = device.queue(index);
            return ReadOutcome::Failed(ffmpeg_next::Error::InvalidData);
        };
        let pts = buf
            .ts_sec
            .saturating_mul(1_000_000)
            .saturating_add(buf.ts_usec)
            .saturating_add(self.clock_offset_us);
        device.in_flight.fetch_add(1, Ordering::SeqCst);
        let release = Box::into_raw(Box::new(Release {
            device: Arc::clone(device),
            index,
        }));
        use ffmpeg_next::codec::packet::Mut;
        let mut packet = ffmpeg_next::Packet::empty();
        // SAFETY: the AVBuffer wraps the mapped (read-only) capture buffer;
        // `release_buffer` takes back `release` exactly once, when the last
        // reference goes, and requeues the buffer. The packet's data lies
        // inside that buffer.
        unsafe {
            let buffer_ref = ffmpeg_next::ffi::av_buffer_create(
                buffer.map as *mut u8,
                bytes,
                Some(release_buffer),
                release as *mut std::ffi::c_void,
                ffmpeg_next::ffi::AV_BUFFER_FLAG_READONLY,
            );
            if buffer_ref.is_null() {
                release_buffer(release as *mut std::ffi::c_void, std::ptr::null_mut());
                return ReadOutcome::Failed(ffmpeg_next::Error::Other {
                    errno: libc_enomem(),
                });
            }
            let raw = packet.as_mut_ptr();
            (*raw).buf = buffer_ref;
            (*raw).data = buffer.map as *mut u8;
            (*raw).size = size;
            (*raw).pts = pts;
            (*raw).dts = pts;
            (*raw).stream_index = 0;
            (*raw).flags |= ffmpeg_next::ffi::AV_PKT_FLAG_KEY;
        }
        let dmabuf = DmabufRef::new(
            buffer.dmabuf.as_raw_fd(),
            self.layout,
            Arc::clone(device) as Arc<dyn Send + Sync>,
        );
        ReadOutcome::Packet(RawPacket::from((packet, Rational(1, 1_000_000))).with_dmabuf(dmabuf))
    }
}

fn libc_enomem() -> i32 {
    Errno::NOMEM.raw_os_error()
}

fn errno(e: Errno) -> ffmpeg_next::Error {
    ffmpeg_next::Error::from(-e.raw_os_error())
}

fn raw_video_parameters(layout: &Layout) -> anyhow::Result<ffmpeg_next::codec::Parameters> {
    let params = ffmpeg_next::codec::Parameters::new();
    // SAFETY: freshly allocated, exclusively owned parameters; plain fields.
    unsafe {
        let p = params.as_ptr() as *mut ffmpeg_next::ffi::AVCodecParameters;
        (*p).codec_type = ffmpeg_next::ffi::AVMediaType::AVMEDIA_TYPE_VIDEO;
        (*p).codec_id = ffmpeg_next::ffi::AVCodecID::AV_CODEC_ID_RAWVIDEO;
        (*p).format = ffmpeg_next::ffi::AVPixelFormat::from(layout.pixel) as i32;
        (*p).width = i32::try_from(layout.width)?;
        (*p).height = i32::try_from(layout.height)?;
    }
    Ok(params)
}

/// Raw V4L2 definitions (videodev2.h), checked against the kernel headers'
/// sizes and ioctl numbers on aarch64 and x86_64.
mod sys {
    use super::*;

    pub const CAP_VIDEO_CAPTURE_MPLANE: u32 = 0x0000_1000;
    pub const CAP_STREAMING: u32 = 0x0400_0000;
    pub const BUF_TYPE_VIDEO_CAPTURE_MPLANE: u32 = 9;
    pub const MEMORY_MMAP: u32 = 1;
    pub const BUF_FLAG_ERROR: u32 = 0x0000_0040;

    #[repr(C)]
    pub struct Capability {
        pub driver: [u8; 16],
        pub card: [u8; 32],
        pub bus_info: [u8; 32],
        pub version: u32,
        pub capabilities: u32,
        pub device_caps: u32,
        pub reserved: [u32; 3],
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct PlanePixFormat {
        pub sizeimage: u32,
        pub bytesperline: u32,
        pub reserved: [u16; 6],
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct PixFormatMplane {
        pub width: u32,
        pub height: u32,
        pub pixelformat: u32,
        pub field: u32,
        pub colorspace: u32,
        pub plane_fmt: [PlanePixFormat; 8],
        pub num_planes: u8,
        pub flags: u8,
        pub ycbcr_enc: u8,
        pub quantization: u8,
        pub xfer_func: u8,
        pub reserved: [u8; 7],
    }

    #[repr(C)]
    pub struct Format {
        pub type_: u32,
        _pad: u32,
        pub pix_mp: PixFormatMplane,
        _rest: [u8; 8],
    }

    impl Format {
        pub fn mplane() -> Self {
            Self {
                type_: BUF_TYPE_VIDEO_CAPTURE_MPLANE,
                _pad: 0,
                pix_mp: PixFormatMplane::default(),
                _rest: [0; 8],
            }
        }
    }

    #[repr(C)]
    #[derive(Default)]
    pub struct RequestBuffers {
        pub count: u32,
        pub type_: u32,
        pub memory: u32,
        pub capabilities: u32,
        pub flags: u8,
        pub reserved: [u8; 3],
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Plane {
        pub bytesused: u32,
        pub length: u32,
        /// Union of mem_offset (low 32 bits), userptr and fd.
        pub m: u64,
        pub data_offset: u32,
        pub reserved: [u32; 11],
    }

    #[repr(C)]
    pub struct Buffer {
        pub index: u32,
        pub type_: u32,
        pub bytesused: u32,
        pub flags: u32,
        pub field: u32,
        _pad0: u32,
        pub ts_sec: i64,
        pub ts_usec: i64,
        pub timecode: [u32; 4],
        pub sequence: u32,
        pub memory: u32,
        pub planes: *mut Plane,
        pub length: u32,
        pub reserved2: u32,
        pub request_fd: u32,
        _pad1: u32,
    }

    impl Buffer {
        pub fn mmap(index: u32, planes: &mut [Plane; 1]) -> Self {
            Self {
                index,
                type_: BUF_TYPE_VIDEO_CAPTURE_MPLANE,
                bytesused: 0,
                flags: 0,
                field: 0,
                _pad0: 0,
                ts_sec: 0,
                ts_usec: 0,
                timecode: [0; 4],
                sequence: 0,
                memory: MEMORY_MMAP,
                planes: planes.as_mut_ptr(),
                length: planes.len() as u32,
                reserved2: 0,
                request_fd: 0,
                _pad1: 0,
            }
        }
    }

    #[repr(C)]
    #[derive(Default)]
    pub struct ExportBuffer {
        pub type_: u32,
        pub index: u32,
        pub plane: u32,
        pub flags: u32,
        pub fd: i32,
        pub reserved: [u32; 11],
    }

    #[repr(C, packed)]
    #[derive(Clone, Copy)]
    pub struct BtTimings {
        pub width: u32,
        pub height: u32,
        pub interlaced: u32,
        pub polarities: u32,
        pub pixelclock: u64,
        pub hfrontporch: u32,
        pub hsync: u32,
        pub hbackporch: u32,
        pub vfrontporch: u32,
        pub vsync: u32,
        pub vbackporch: u32,
        pub il_vfrontporch: u32,
        pub il_vsync: u32,
        pub il_vbackporch: u32,
        pub rest: [u8; 64],
    }

    #[repr(C, packed)]
    pub struct DvTimings {
        pub type_: u32,
        pub bt: BtTimings,
        pub reserved: [u8; 4],
    }

    const _: () = assert!(std::mem::size_of::<Capability>() == 104);
    const _: () = assert!(std::mem::size_of::<Format>() == 208);
    const _: () = assert!(std::mem::size_of::<PixFormatMplane>() == 192);
    const _: () = assert!(std::mem::size_of::<RequestBuffers>() == 20);
    const _: () = assert!(std::mem::size_of::<Plane>() == 64);
    const _: () = assert!(std::mem::size_of::<Buffer>() == 88);
    const _: () = assert!(std::mem::size_of::<ExportBuffer>() == 64);
    const _: () = assert!(std::mem::size_of::<BtTimings>() == 124);
    const _: () = assert!(std::mem::size_of::<DvTimings>() == 132);

    use rustix::ioctl::{Getter, Updater, opcode};
    const V: u8 = b'V';
    pub type Querycap = Getter<{ opcode::read::<Capability>(V, 0) }, Capability>;
    pub type GFmt<'a> = Updater<'a, { opcode::read_write::<Format>(V, 4) }, Format>;
    pub type Reqbufs<'a> =
        Updater<'a, { opcode::read_write::<RequestBuffers>(V, 8) }, RequestBuffers>;
    pub type Querybuf<'a> = Updater<'a, { opcode::read_write::<Buffer>(V, 9) }, Buffer>;
    pub type Qbuf<'a> = Updater<'a, { opcode::read_write::<Buffer>(V, 15) }, Buffer>;
    pub type Expbuf<'a> = Updater<'a, { opcode::read_write::<ExportBuffer>(V, 16) }, ExportBuffer>;
    pub type Dqbuf<'a> = Updater<'a, { opcode::read_write::<Buffer>(V, 17) }, Buffer>;
    pub type GDvTimings<'a> = Updater<'a, { opcode::read_write::<DvTimings>(V, 88) }, DvTimings>;
    const STREAMON: rustix::ioctl::Opcode = opcode::write::<i32>(V, 18);
    const STREAMOFF: rustix::ioctl::Opcode = opcode::write::<i32>(V, 19);

    // Ioctl numbers as the kernel headers define them (see the module docs).
    const _: () = assert!(opcode::read::<Capability>(V, 0) == 0x8068_5600);
    const _: () = assert!(opcode::read_write::<Format>(V, 4) == 0xc0d0_5604);
    const _: () = assert!(opcode::read_write::<RequestBuffers>(V, 8) == 0xc014_5608);
    const _: () = assert!(opcode::read_write::<Buffer>(V, 9) == 0xc058_5609);
    const _: () = assert!(opcode::read_write::<Buffer>(V, 15) == 0xc058_560f);
    const _: () = assert!(opcode::read_write::<ExportBuffer>(V, 16) == 0xc040_5610);
    const _: () = assert!(opcode::read_write::<Buffer>(V, 17) == 0xc058_5611);
    const _: () = assert!(STREAMON == 0x4004_5612);
    const _: () = assert!(STREAMOFF == 0x4004_5613);
    const _: () = assert!(opcode::read_write::<DvTimings>(V, 88) == 0xc084_5658);

    pub fn stream(fd: &OwnedFd, on: bool) -> Result<(), Errno> {
        let kind = BUF_TYPE_VIDEO_CAPTURE_MPLANE as i32;
        // SAFETY: STREAMON/STREAMOFF read one int buffer type.
        unsafe {
            if on {
                rustix::ioctl::ioctl(
                    fd.as_fd(),
                    rustix::ioctl::Setter::<STREAMON, i32>::new(kind),
                )
            } else {
                rustix::ioctl::ioctl(
                    fd.as_fd(),
                    rustix::ioctl::Setter::<STREAMOFF, i32>::new(kind),
                )
            }
        }
    }

    pub fn dv_frame_rate(fd: &OwnedFd) -> Option<Rational> {
        // SAFETY: DvTimings is plain old data; all-zero is valid input.
        let mut timings: DvTimings = unsafe { std::mem::zeroed() };
        // SAFETY: `timings` is a valid v4l2_dv_timings for the driver to fill.
        unsafe { rustix::ioctl::ioctl(fd.as_fd(), GDvTimings::new(&mut timings)) }.ok()?;
        let bt = timings.bt;
        let total_width = u64::from(bt.width)
            + u64::from(bt.hfrontporch)
            + u64::from(bt.hsync)
            + u64::from(bt.hbackporch);
        let mut total_height = u64::from(bt.height)
            + u64::from(bt.vfrontporch)
            + u64::from(bt.vsync)
            + u64::from(bt.vbackporch);
        if bt.interlaced != 0 {
            total_height +=
                u64::from(bt.il_vfrontporch) + u64::from(bt.il_vsync) + u64::from(bt.il_vbackporch);
        }
        frame_rate_from_timings(bt.pixelclock, total_width, total_height)
    }

    pub fn realtime_minus_monotonic_us() -> i64 {
        use rustix::time::{ClockId, clock_gettime};
        let us = |t: rustix::time::Timespec| t.tv_sec * 1_000_000 + t.tv_nsec / 1000;
        us(clock_gettime(ClockId::Realtime)) - us(clock_gettime(ClockId::Monotonic))
    }

    pub fn c_str(bytes: &[u8]) -> String {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    }
}

#[cfg(test)]
#[path = "v4l2_dmabuf_test.rs"]
mod v4l2_dmabuf_test;
