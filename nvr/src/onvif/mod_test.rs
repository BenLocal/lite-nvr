use super::*;

fn cfg(host: &str) -> OnvifConfig {
    OnvifConfig {
        host: host.into(),
        port: 80,
        username: "u".into(),
        password: "p".into(),
        profile_token: None,
    }
}

#[test]
fn register_get_remove() {
    register("dev-a", cfg("10.0.0.1"));
    assert_eq!(get("dev-a").unwrap().host, "10.0.0.1");
    remove("dev-a");
    assert!(get("dev-a").is_none());
}
