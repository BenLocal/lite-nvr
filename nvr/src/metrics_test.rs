use super::{device_tree_string, os_info};

/// Device-tree strings are NUL-terminated and may be padded.
#[test]
fn test_device_tree_string() {
    assert_eq!(
        device_tree_string(b"Rockchip RK3588 EVB1 LP4 V10 Board\0").as_deref(),
        Some("Rockchip RK3588 EVB1 LP4 V10 Board")
    );
    assert_eq!(
        device_tree_string(b"rk3588\0\0\0").as_deref(),
        Some("rk3588")
    );
    assert_eq!(device_tree_string(b"\0"), None);
    assert_eq!(device_tree_string(b""), None);
}

/// Collected once: repeated reads return the same snapshot.
#[test]
fn test_os_info_is_collected_once() {
    let first = os_info();
    assert!(!first.arch.is_empty());
    assert!(first.cpu_core_count > 0);
    assert!(first.mem_total > 0);
    assert!(first.boot_time > 0);
    assert_eq!(first.nvr_version, env!("CARGO_PKG_VERSION"));
    let second = os_info();
    assert_eq!(first.boot_time, second.boot_time);
    assert_eq!(first.host_name, second.host_name);
}
