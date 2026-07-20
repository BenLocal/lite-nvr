use super::*;

#[test]
fn extracts_probe_message_id() {
    let probe = r#"<e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope"
 xmlns:w="http://schemas.xmlsoap.org/ws/2004/08/addressing"><e:Header>
<w:MessageID>urn:uuid:abc-123</w:MessageID></e:Header><e:Body/></e:Envelope>"#;
    assert_eq!(
        extract_message_id(probe).as_deref(),
        Some("urn:uuid:abc-123")
    );
}

#[test]
fn probe_matches_carries_xaddr_and_scopes() {
    let x = probe_matches_xml(
        "id-1",
        "urn:uuid:abc-123",
        "http://127.0.0.1:8000/onvif/device_service",
        "dummy-model",
        "lite-nvr",
    );
    assert!(x.contains("<d:XAddrs>http://127.0.0.1:8000/onvif/device_service</d:XAddrs>"));
    assert!(x.contains("onvif://www.onvif.org/name/dummy-model"));
    assert!(x.contains("onvif://www.onvif.org/hardware/lite-nvr"));
    assert!(x.contains("<w:RelatesTo>urn:uuid:abc-123</w:RelatesTo>"));
    assert!(x.contains("ProbeMatches"));
}
