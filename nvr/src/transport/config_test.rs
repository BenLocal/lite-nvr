use super::*;

#[test]
fn remote_key_joins_and_trims_slashes() {
    assert_eq!(
        remote_key("nvr/records", "cam1", "a.ts"),
        "nvr/records/cam1/a.ts"
    );
    assert_eq!(remote_key("/nvr/", "/cam1/", "a.ts"), "nvr/cam1/a.ts");
    assert_eq!(remote_key("", "cam1", "a.ts"), "cam1/a.ts");
    assert_eq!(remote_key("", "", "a.ts"), "a.ts");
}

#[test]
fn redact_blanks_only_password() {
    let out = redact_config(r#"{"host":"h","password":"secret","base_path":"p"}"#);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["password"], "");
    assert_eq!(v["host"], "h");
    assert_eq!(v["base_path"], "p");
}

#[test]
fn redact_passthrough_on_non_json() {
    assert_eq!(redact_config("not json"), "not json");
}

#[test]
fn routing_distinguishes_legacy_all_from_no_devices_and_rejects_bad_types() {
    assert!(
        serde_json::from_str::<TransportRouting>("{}")
            .unwrap()
            .stream_ids
            .is_none()
    );
    assert!(
        serde_json::from_str::<TransportRouting>(r#"{"stream_ids":null}"#)
            .unwrap()
            .stream_ids
            .is_none()
    );
    assert_eq!(
        serde_json::from_str::<TransportRouting>(r#"{"stream_ids":[]}"#)
            .unwrap()
            .stream_ids,
        Some(vec![])
    );
    assert!(serde_json::from_str::<TransportRouting>(r#"{"stream_ids":"cam"}"#).is_err());
}

#[test]
fn rejects_incomplete_destinations_and_blank_device_ids() {
    for invalid in [
        serde_json::json!({"host":""}),
        serde_json::json!({"host":"server", "port":0}),
        serde_json::json!({"host":"server", "stream_ids":[""]}),
        serde_json::json!({"host":"server", "stream_ids":[1]}),
    ] {
        assert!(validate_config("ftp", &invalid).is_err());
    }
    assert!(validate_config("smb", &serde_json::json!({"host":"server", "share":"/"})).is_err());
    assert!(
        validate_config(
            "ftp",
            &serde_json::json!({"host":"server", "stream_ids":["cam"]})
        )
        .is_ok()
    );
    assert!(
        validate_config(
            "smb",
            &serde_json::json!({"host":"server", "share":"records"})
        )
        .is_ok()
    );
}
