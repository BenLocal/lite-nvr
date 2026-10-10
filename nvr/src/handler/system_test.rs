use std::path::PathBuf;

use super::sort_video_paths;

/// video10 sorts after video2, not between video1 and video2.
#[test]
fn test_sort_video_paths_is_numeric() {
    let mut paths: Vec<PathBuf> = ["/dev/video10", "/dev/video2", "/dev/video0", "/dev/video1"]
        .iter()
        .map(PathBuf::from)
        .collect();
    sort_video_paths(&mut paths);
    let names: Vec<_> = paths.iter().map(|p| p.display().to_string()).collect();
    assert_eq!(
        names,
        ["/dev/video0", "/dev/video1", "/dev/video2", "/dev/video10"]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn test_has_cap_prefers_per_node_caps() {
    use tokio_linux_video::types::CapabilityFlag;

    use super::has_cap;

    let capture = CapabilityFlag::VideoCapture;
    let device =
        CapabilityFlag::VideoCapture | CapabilityFlag::MetaCapture | CapabilityFlag::DeviceCaps;
    // A UVC metadata node: the device can capture, this node cannot.
    assert!(!has_cap(device, CapabilityFlag::MetaCapture, capture));
    assert!(has_cap(device, CapabilityFlag::VideoCapture, capture));
    // Rockchip rkcif / hdmirx report 0x84201000: multi-planar capture only,
    // which FFmpeg's v4l2 input cannot read.
    let rk = CapabilityFlag::VideoCaptureMplane | CapabilityFlag::DeviceCaps;
    assert!(!has_cap(rk, CapabilityFlag::VideoCaptureMplane, capture));
    assert!(has_cap(
        rk,
        CapabilityFlag::VideoCaptureMplane,
        CapabilityFlag::VideoCaptureMplane
    ));
    // Without DeviceCaps the device-wide caps are all there is.
    assert!(has_cap(
        CapabilityFlag::VideoCapture,
        CapabilityFlag::none(),
        capture
    ));
}
