use super::*;

#[test]
fn injects_into_plain_rtsp() {
    assert_eq!(
        inject_credentials("rtsp://192.168.1.5:554/Streaming/1", "admin", "pass"),
        "rtsp://admin:pass@192.168.1.5:554/Streaming/1"
    );
}

#[test]
fn percent_encodes_special_chars() {
    assert_eq!(
        inject_credentials("rtsp://cam/live", "adm in", "p@ss:1"),
        "rtsp://adm%20in:p%40ss%3A1@cam/live"
    );
}

#[test]
fn leaves_existing_credentials_untouched() {
    let u = "rtsp://user:pw@cam/live";
    assert_eq!(inject_credentials(u, "admin", "x"), u);
}

#[test]
fn passes_through_empty_user_or_non_rtsp() {
    assert_eq!(
        inject_credentials("rtsp://cam/live", "", "x"),
        "rtsp://cam/live"
    );
    assert_eq!(inject_credentials("http://cam/x", "a", "b"), "http://cam/x");
}
