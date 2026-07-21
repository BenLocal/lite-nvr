use super::*;
use crate::auth::password_digest;
use crate::config::DeviceCfg;

fn cfg() -> DeviceCfg {
    DeviceCfg {
        host: "127.0.0.1".into(),
        port: 8000,
        username: "admin".into(),
        password: "secret".into(),
        rtsp_url: "rtsp://127.0.0.1:9554/live/test1".into(),
        manufacturer: "lite-nvr".into(),
        model: "dummy".into(),
        firmware: "0.1".into(),
        serial: "SN-0001".into(),
    }
}

/// Build a SOAP request with a WS-Security header for `op`.
fn req(op: &str, user: &str, pass: &str) -> String {
    let nonce = "MTIzNDU2Nzg5MDEyMzQ1Ng==";
    let created = "2026-07-20T00:00:00Z";
    let digest = password_digest(nonce, created, pass);
    format!(
        r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"
 xmlns:w="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd"
 xmlns:u="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd">
<s:Header><w:Security><w:UsernameToken>
<w:Username>{user}</w:Username>
<w:Password Type="...#PasswordDigest">{digest}</w:Password>
<w:Nonce>{nonce}</w:Nonce><u:Created>{created}</u:Created>
</w:UsernameToken></w:Security></s:Header>
<s:Body><trt:{op} xmlns:trt="http://www.onvif.org/ver10/media/wsdl"/></s:Body></s:Envelope>"#
    )
}

#[test]
fn detects_operations() {
    assert_eq!(
        detect_op(&req("GetProfiles", "a", "b")),
        Some("GetProfiles")
    );
    assert_eq!(
        detect_op(&req("GetStreamUri", "a", "b")),
        Some("GetStreamUri")
    );
    assert_eq!(detect_op("<x/>"), None);
}

#[test]
fn valid_auth_returns_200_and_op_response() {
    let r = handle(&req("GetStreamUri", "admin", "secret"), &cfg());
    assert_eq!(r.status, 200);
    assert!(r.body.contains("rtsp://127.0.0.1:9554/live/test1"));
}

#[test]
fn wrong_password_returns_400_not_authorized() {
    let r = handle(&req("GetProfiles", "admin", "WRONG"), &cfg());
    assert_eq!(r.status, 400);
    assert!(r.body.contains("NotAuthorized"));
}

#[test]
fn missing_security_returns_400() {
    let no_hdr = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body>
<trt:GetProfiles xmlns:trt="http://www.onvif.org/ver10/media/wsdl"/></s:Body></s:Envelope>"#;
    let r = handle(no_hdr, &cfg());
    assert_eq!(r.status, 400);
    assert!(r.body.contains("NotAuthorized"));
}

#[test]
fn unknown_op_returns_fault() {
    let unknown = r#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body>
<trt:GetSomethingElse xmlns:trt="x"/></s:Body></s:Envelope>"#;
    let r = handle(unknown, &cfg());
    assert_eq!(r.status, 500);
    assert!(r.body.contains("ActionNotSupported"));
}
