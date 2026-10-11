use super::local_source_url;

#[test]
fn local_source_uses_configured_rtsp_port_and_preserves_external_urls() -> anyhow::Result<()> {
    let port = crate::zlm::server::rtsp_port()?;
    assert_eq!(
        local_source_url("rtsp://127.0.0.1:8554/device/camera")?,
        format!("rtsp://127.0.0.1:{port}/device/camera")
    );
    assert_eq!(
        local_source_url("rtsp://camera.example:8554/device/camera")?,
        "rtsp://camera.example:8554/device/camera"
    );
    assert_eq!(
        local_source_url("rtsp://127.0.0.1:9554/dummy")?,
        "rtsp://127.0.0.1:9554/dummy"
    );
    Ok(())
}
