use std::collections::HashMap;

use super::*;

fn resolved(url: &str, headers: &[(&str, &str)]) -> ResolvedStream {
    ResolvedStream {
        url: url.to_string(),
        http_headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        is_live: true,
        title: None,
        protocol: None,
    }
}

#[test]
fn http_headers_forwarded_to_demuxer() {
    let opts = input_options(&resolved(
        "https://pull-cdn.example.com/live/x.flv",
        &[("Referer", "https://live.douyin.com/")],
    ))
    .unwrap();
    assert_eq!(
        opts.get("headers").map(String::as_str),
        Some("Referer: https://live.douyin.com/\r\n")
    );
}

#[test]
fn http_without_headers_needs_no_options() {
    assert!(input_options(&resolved("https://cdn/x.m3u8", &[])).is_none());
}

#[test]
fn rtsp_gets_tcp_transport_policy() {
    let opts = input_options(&resolved("rtsp://host/stream", &[])).unwrap();
    assert_eq!(opts.get("rtsp_transport").map(String::as_str), Some("tcp"));
    assert_eq!(opts.get("stimeout").map(String::as_str), Some("5000000"));
}

#[test]
fn other_protocols_get_no_options() {
    assert!(input_options(&resolved("rtmp://cdn/live/x", &[])).is_none());
}

#[test]
fn multiple_headers_joined_crlf() {
    let opts = input_options(&resolved("http://cdn/x.flv", &[("A", "1"), ("B", "2")])).unwrap();
    let headers = opts.get("headers").unwrap();
    assert!(headers.contains("A: 1\r\n"), "{headers}");
    assert!(headers.contains("B: 2\r\n"), "{headers}");
    let _: &HashMap<String, String> = &opts;
}
