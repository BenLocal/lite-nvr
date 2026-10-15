use std::sync::Arc;

use ffmpeg_next::Rational;
use ffmpeg_next::format::Pixel;

use super::*;

fn layout(pixel: Pixel, bytes_per_line: u32) -> Layout {
    Layout {
        width: 1920,
        height: 1080,
        pixel,
        bytes_per_line,
        size: bytes_per_line * 1080 * if pixel == Pixel::NV12 { 3 } else { 2 } / 2,
    }
}

#[test]
fn enabled_only_when_explicitly_set() {
    assert!(enabled_from(Some("1")));
    assert!(enabled_from(Some(" 1 ")));
    assert!(!enabled_from(None));
    assert!(!enabled_from(Some("0")));
    assert!(!enabled_from(Some("true")));
}

#[test]
fn frame_rate_from_cea_1080p60_timings() {
    // CEA-861 1080p60: 148.5 MHz over 2200x1125.
    assert_eq!(
        frame_rate_from_timings(148_500_000, 2200, 1125),
        Some(Rational(60, 1))
    );
    // 1080p59.94: 148.35 MHz.
    assert_eq!(
        frame_rate_from_timings(148_351_648, 2200, 1125),
        Some(Rational(5994, 100).reduce())
    );
    assert_eq!(frame_rate_from_timings(0, 2200, 1125), None);
    assert_eq!(frame_rate_from_timings(148_500_000, 0, 1125), None);
}

#[test]
fn drm_descriptor_describes_bgr24_as_one_plane() {
    let layout = layout(Pixel::BGR24, 5760);
    let desc = drm_descriptor(7, &layout);
    assert_eq!(desc.nb_objects, 1);
    assert_eq!(desc.objects[0].fd, 7);
    assert_eq!(desc.objects[0].size, 6_220_800);
    assert_eq!(desc.objects[0].format_modifier, 0);
    assert_eq!(desc.nb_layers, 1);
    assert_eq!(desc.layers[0].nb_planes, 1);
    assert_eq!(desc.layers[0].planes[0].pitch, 5760);
    assert_eq!(desc.layers[0].format, u32::from_le_bytes(*b"RG24"));
}

#[test]
fn drm_descriptor_places_nv12_chroma_after_luma() {
    let layout = layout(Pixel::NV12, 1920);
    let desc = drm_descriptor(9, &layout);
    assert_eq!(desc.objects[0].size, 3_110_400);
    assert_eq!(desc.layers[0].nb_planes, 2);
    assert_eq!(desc.layers[0].planes[0].pitch, 1920);
    assert_eq!(desc.layers[0].planes[1].offset, 1920 * 1080);
    assert_eq!(desc.layers[0].planes[1].pitch, 1920);
}

/// The DRM frame keeps the capture buffer referenced until the encoder (or
/// anyone else) drops the frame.
#[test]
fn drm_frame_holds_the_capture_buffer() {
    let keep: Arc<dyn Send + Sync> = Arc::new(());
    let dmabuf = DmabufRef::new(11, layout(Pixel::NV12, 1920), Arc::clone(&keep));
    let frame = drm_frame(&dmabuf, Some(42)).unwrap();
    drop(dmabuf);
    assert_eq!(frame.format(), Pixel::DRM_PRIME);
    assert_eq!((frame.width(), frame.height()), (1920, 1080));
    assert_eq!(frame.pts(), Some(42));
    // SAFETY: data[0] of a DRM_PRIME frame is its AVDRMFrameDescriptor.
    let fd = unsafe {
        (*((*frame.as_ptr()).data[0] as *const ffmpeg_next::ffi::AVDRMFrameDescriptor)).objects[0]
            .fd
    };
    assert_eq!(fd, 11);
    assert_eq!(Arc::strong_count(&keep), 2, "frame keeps the buffer");
    drop(frame);
    assert_eq!(Arc::strong_count(&keep), 1, "released with the frame");
}

/// A raw-video frame built from a capture packet carries its DMABUF; pixel
/// writes (which copy) drop it, since the pixels then differ from the buffer.
#[test]
fn raw_frame_carries_dmabuf_until_pixels_change() {
    let data = vec![0u8; 6 * 4 * 3 / 2];
    let keep: Arc<dyn Send + Sync> = Arc::new(());
    let packet = crate::packet::RawPacket::from((
        ffmpeg_next::codec::packet::Packet::copy(&data),
        Rational(1, 1_000_000),
    ))
    .with_dmabuf(DmabufRef::new(5, layout(Pixel::NV12, 1920), keep));
    let crate::frame::RawFrame::Video(mut frame) =
        crate::frame::packet_to_raw_video_frame(packet, 6, 4, Pixel::NV12).unwrap()
    else {
        panic!("expected video")
    };
    assert_eq!(frame.dmabuf().map(DmabufRef::fd), Some(5));
    frame.set_pts(Some(1));
    assert!(frame.dmabuf().is_some(), "property changes keep the buffer");
    frame.get_mut().data_mut(0)[0] = 1;
    assert!(
        frame.dmabuf().is_none(),
        "rewritten pixels are not the buffer"
    );
}

#[test]
fn non_v4l2_path_is_rejected_without_panicking() {
    let err = DmabufCapture::open("/dev/null", None)
        .err()
        .expect("not a capture device");
    assert!(err.to_string().contains("V4L2"), "{err:#}");
    assert!(DmabufCapture::open("/nonexistent/video", None).is_err());
}

/// On a Rockchip host: `FFMPEG_BUS_RK_TEST_V4L2=/dev/video11` captures a few
/// frames, each backed by a DMABUF.
#[test]
fn rockchip_capture_yields_dmabuf_packets() -> anyhow::Result<()> {
    let Ok(device) = std::env::var("FFMPEG_BUS_RK_TEST_V4L2") else {
        return Ok(());
    };
    let mut capture = DmabufCapture::open(&device, None)?;
    let mut packets = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while packets.len() < 10 && std::time::Instant::now() < deadline {
        match capture.read() {
            ReadOutcome::Packet(packet) => packets.push(packet),
            ReadOutcome::Again => {}
            ReadOutcome::End | ReadOutcome::Failed(_) => anyhow::bail!("capture ended"),
        }
    }
    assert_eq!(packets.len(), 10);
    for packet in &packets {
        let dmabuf = packet.dmabuf().expect("dmabuf");
        assert!(dmabuf.fd() >= 0);
        assert!(packet.size() as u32 <= dmabuf.layout().size);
    }
    Ok(())
}
