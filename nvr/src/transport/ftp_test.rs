use super::*;
use crate::transport::backend::StorageBackend;

/// Real FTP round-trip against a local server. Ignored by default; run with a
/// pyftpdlib server up (see the test scaffolding), e.g.:
///
/// ```text
/// FTP_TEST_PORT=2121 FTP_TEST_ROOT=/tmp/ftproot FTP_TEST_LOCAL=/tmp/ftplocal \
///   cargo test -p nvr --bins --ignored ftp_roundtrip
/// ```
#[tokio::test]
#[ignore]
async fn ftp_roundtrip_uploads_into_nested_dirs() {
    let port = std::env::var("FTP_TEST_PORT").unwrap();
    let root = std::env::var("FTP_TEST_ROOT").unwrap();
    let local_dir = std::env::var("FTP_TEST_LOCAL").unwrap();

    let cfg_json =
        format!(r#"{{"host":"127.0.0.1","port":{port},"username":"user","password":"12345"}}"#);
    let backend = FtpBackend::from_json(&cfg_json).unwrap();

    let local = std::path::Path::new(&local_dir).join("src.ts");
    let payload = b"hello-ftp-transport-roundtrip";
    std::fs::write(&local, payload).unwrap();

    backend
        .upload(&local, "cam1/2026/seg-001.ts")
        .await
        .expect("upload should succeed");

    let landed = std::path::Path::new(&root).join("cam1/2026/seg-001.ts");
    let got = std::fs::read(&landed).expect("file should have landed on the server");
    assert_eq!(got, payload, "uploaded bytes must match");
}
