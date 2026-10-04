use super::*;

#[test]
fn editing_routing_preserves_redacted_password() {
    let merged = merge_password(
        r#"{"host":"old","password":"test-placeholder","stream_ids":null}"#,
        serde_json::json!({"host":"new", "password":"", "stream_ids":["cam"]}),
    );
    let value: serde_json::Value = serde_json::from_str(&merged).unwrap();
    assert_eq!(value["password"], "test-placeholder");
    assert_eq!(value["host"], "new");
    assert_eq!(value["stream_ids"], serde_json::json!(["cam"]));
}

#[test]
fn unsupported_smb_can_be_configured_but_not_enabled() {
    let mut payload = TargetPayload {
        name: "archive".into(),
        kind: "smb".into(),
        enabled: false,
        config: serde_json::json!({"host":"server", "share":"records"}),
        remark: String::new(),
    };
    assert!(validate_payload(&payload).is_ok());
    payload.enabled = true;
    assert_eq!(validate_payload(&payload).is_ok(), cfg!(feature = "smb"));
}
