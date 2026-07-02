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
