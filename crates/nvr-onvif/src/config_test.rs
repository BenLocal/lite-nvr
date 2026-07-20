use super::*;

#[test]
fn service_url_uses_host_port() {
    let c = OnvifConfig {
        host: "192.168.1.50".into(),
        port: 8000,
        username: "admin".into(),
        password: "x".into(),
        profile_token: None,
    };
    assert_eq!(
        c.service_url(),
        "http://192.168.1.50:8000/onvif/device_service"
    );
}

#[test]
fn config_serde_round_trip() {
    let json =
        r#"{"host":"h","port":80,"username":"u","password":"p","profile_token":"Profile_1"}"#;
    let c: OnvifConfig = serde_json::from_str(json).unwrap();
    assert_eq!(c.port, 80);
    assert_eq!(c.profile_token.as_deref(), Some("Profile_1"));
    // profile_token defaults to None when absent
    let c2: OnvifConfig =
        serde_json::from_str(r#"{"host":"h","port":80,"username":"u","password":"p"}"#).unwrap();
    assert_eq!(c2.profile_token, None);
}
