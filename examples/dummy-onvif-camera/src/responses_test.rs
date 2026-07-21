use super::*;
use crate::config::DeviceCfg;

fn cfg() -> DeviceCfg {
    DeviceCfg {
        host: "127.0.0.1".into(),
        port: 8000,
        username: "admin".into(),
        password: "admin".into(),
        rtsp_url: "rtsp://127.0.0.1:9554/live/test1".into(),
        manufacturer: "lite-nvr".into(),
        model: "dummy".into(),
        firmware: "0.1".into(),
        serial: "SN-0001".into(),
    }
}

#[test]
fn capabilities_advertise_media_and_ptz_at_service_url() {
    let x = get_capabilities("http://127.0.0.1:8000/onvif/device_service");
    // both Media and PTZ XAddr point at the service url
    assert_eq!(
        x.matches("http://127.0.0.1:8000/onvif/device_service")
            .count(),
        2
    );
    assert!(x.contains("GetCapabilitiesResponse"));
}

#[test]
fn stream_uri_contains_rtsp_url() {
    let x = get_stream_uri("rtsp://cam/live");
    assert!(x.contains("<tt:Uri>rtsp://cam/live</tt:Uri>"));
}

#[test]
fn profiles_carry_token_and_resolution() {
    let x = get_profiles();
    assert!(x.contains(r#"token="Profile_1""#));
    assert!(x.contains("<tt:Width>1920</tt:Width>"));
    assert!(x.contains("H264"));
}

#[test]
fn device_information_uses_cfg() {
    let x = device_information(&cfg());
    assert!(x.contains("<tds:Manufacturer>lite-nvr</tds:Manufacturer>"));
    assert!(x.contains("<tds:SerialNumber>SN-0001</tds:SerialNumber>"));
}

#[test]
fn not_authorized_fault_has_subcode() {
    assert!(fault_not_authorized().contains("NotAuthorized"));
}

#[test]
fn presets_have_two_tokens() {
    let x = get_presets();
    assert!(x.contains(r#"token="Preset_1""#));
    assert!(x.contains(r#"token="Preset_2""#));
}
