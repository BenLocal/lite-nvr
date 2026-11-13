use super::switcher_publish_url;

#[test]
fn publish_url_uses_the_configured_rtmp_port() {
    assert_eq!(
        switcher_publish_url(8557, "studio"),
        "rtmp://127.0.0.1:8557/switcher/studio"
    );
}
