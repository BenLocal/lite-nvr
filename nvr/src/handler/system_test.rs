use std::path::PathBuf;

use super::{sort_video_paths, x11_displays};

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

fn display_names(sockets: &[&str], current: Option<&str>) -> Vec<(String, bool)> {
    let sockets: Vec<String> = sockets.iter().map(|s| s.to_string()).collect();
    x11_displays(&sockets, current)
        .into_iter()
        .map(|d| (d.display, d.current))
        .collect()
}

/// Every X server socket is a display; DISPLAY comes first and is not repeated.
#[test]
fn test_x11_displays_from_sockets_and_display_env() {
    assert_eq!(
        display_names(&["X99", "X0", "X1"], Some(":0")),
        [
            (":0".into(), true),
            (":1".into(), false),
            (":99".into(), false)
        ]
    );
    // ":0.0" names the same server as socket X0.
    assert_eq!(
        display_names(&["X0"], Some(":0.0")),
        [(":0.0".into(), true)]
    );
    // Non-display entries are ignored.
    assert_eq!(
        display_names(&["X1", "lock", "Xabc"], None),
        [(":1".into(), false)]
    );
}

#[test]
fn test_x11_displays_fall_back_to_zero() {
    assert_eq!(display_names(&[], None), [(":0".into(), false)]);
    assert_eq!(display_names(&[], Some("  ")), [(":0".into(), false)]);
}
