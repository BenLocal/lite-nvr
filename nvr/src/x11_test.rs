use super::*;

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
fn test_x11_displays_do_not_invent_a_display() {
    assert!(display_names(&[], None).is_empty());
    assert!(display_names(&[], Some("  ")).is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn x11_probe_requires_a_frame_and_bounds_hangs() {
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join(format!("nvr-x11-probe-{}", uuid::Uuid::new_v4()));
    for (script, expected) in [
        ("printf abc", true),
        ("exit 0", false),
        ("exit 1", false),
        ("exec sleep 10", false),
    ] {
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            probe(
                &path,
                ":0",
                if script.starts_with("exec sleep") {
                    Duration::from_millis(200)
                } else {
                    Duration::from_secs(3)
                }
            )
            .await,
            expected
        );
    }
    std::fs::remove_file(path).unwrap();
}
