use super::*;

#[test]
fn service_url_format() {
    let (cfg, _) = Args::parse_from(["x", "--host", "10.0.0.9", "--port", "8899"]).into_cfg();
    assert_eq!(
        cfg.service_url(),
        "http://10.0.0.9:8899/onvif/device_service"
    );
}

#[test]
fn defaults_and_toggles() {
    let (cfg, opts) = Args::parse_from(["x"]).into_cfg();
    assert_eq!(cfg.host, "127.0.0.1");
    assert_eq!(cfg.port, 8000);
    assert_eq!(cfg.username, "admin");
    assert_eq!(cfg.password, "admin");
    assert_eq!(cfg.rtsp_url, "rtsp://127.0.0.1:9554/live/test1");
    assert!(opts.discovery); // on unless --no-discovery
    assert!(!opts.launch_rtsp);

    let (_, opts2) = Args::parse_from(["x", "--no-discovery", "--launch-rtsp"]).into_cfg();
    assert!(!opts2.discovery);
    assert!(opts2.launch_rtsp);
}
