//! Minimal GB-dialect SDP for Play (offer we send) + parse of a device's answer.

use std::net::SocketAddr;

use crate::error::{GbError, Result};
use crate::types::{StreamType, Transport};

/// Build the SDP offer for a Play/Playback INVITE.
/// `local_id` is our platform GB id (used in `o=`).
pub fn build_play_offer(
    local_id: &str,
    recv_ip: &str,
    recv_port: u16,
    ssrc_str: &str,
    transport: Transport,
    stream_type: &StreamType,
) -> String {
    let session = match stream_type {
        StreamType::Play => "Play",
        StreamType::Playback { .. } => "Playback",
        StreamType::Download => "Download",
    };
    let proto = match transport {
        Transport::Udp => "RTP/AVP",
        Transport::TcpPassive | Transport::TcpActive => "TCP/RTP/AVP",
    };
    let mut sdp = String::new();
    sdp.push_str("v=0\r\n");
    sdp.push_str(&format!("o={local_id} 0 0 IN IP4 {recv_ip}\r\n"));
    sdp.push_str(&format!("s={session}\r\n"));
    sdp.push_str(&format!("c=IN IP4 {recv_ip}\r\n"));
    if let StreamType::Playback { start, end } = stream_type {
        sdp.push_str(&format!("t={start} {end}\r\n"));
    } else {
        sdp.push_str("t=0 0\r\n");
    }
    sdp.push_str(&format!("m=video {recv_port} {proto} 96 98\r\n"));
    sdp.push_str("a=recvonly\r\n");
    sdp.push_str("a=rtpmap:96 PS/90000\r\n");
    sdp.push_str("a=rtpmap:98 H264/90000\r\n");
    if matches!(transport, Transport::TcpPassive) {
        sdp.push_str("a=setup:passive\r\n");
        sdp.push_str("a=connection:new\r\n");
    } else if matches!(transport, Transport::TcpActive) {
        sdp.push_str("a=setup:active\r\n");
        sdp.push_str("a=connection:new\r\n");
    }
    sdp.push_str(&format!("y={ssrc_str}\r\n"));
    sdp
}

/// Parsed fields we care about from a device's answer SDP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerSdp {
    pub media_addr: SocketAddr,
    pub ssrc: Option<String>,
}

/// Parse the `c=`/`m=` (address+port) and `y=` (ssrc) from an answer.
pub fn parse_answer(sdp: &str) -> Result<AnswerSdp> {
    let mut ip: Option<&str> = None;
    let mut port: Option<u16> = None;
    let mut ssrc: Option<String> = None;
    for line in sdp.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("c=IN IP4 ") {
            ip = Some(rest.trim());
        } else if let Some(rest) = line.strip_prefix("m=video ") {
            port = rest.split_whitespace().next().and_then(|p| p.parse().ok());
        } else if let Some(rest) = line.strip_prefix("y=") {
            ssrc = Some(rest.trim().to_string());
        }
    }
    let ip = ip.ok_or_else(|| GbError::Sdp("missing c= line".into()))?;
    let port = port.ok_or_else(|| GbError::Sdp("missing m=video port".into()))?;
    let media_addr: SocketAddr = format!("{ip}:{port}")
        .parse()
        .map_err(|e| GbError::Sdp(format!("bad addr {ip}:{port}: {e}")))?;
    Ok(AnswerSdp { media_addr, ssrc })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_offer_has_gb_lines() {
        let sdp = build_play_offer(
            "34020000002000000001",
            "192.168.1.10",
            30000,
            "0200000001",
            Transport::Udp,
            &StreamType::Play,
        );
        assert!(sdp.contains("s=Play\r\n"));
        assert!(sdp.contains("m=video 30000 RTP/AVP 96 98\r\n"));
        assert!(sdp.contains("y=0200000001\r\n"));
        assert!(sdp.contains("a=recvonly\r\n"));
    }

    #[test]
    fn tcp_passive_offer_sets_setup() {
        let sdp = build_play_offer(
            "id",
            "10.0.0.1",
            40000,
            "0200000002",
            Transport::TcpPassive,
            &StreamType::Play,
        );
        assert!(sdp.contains("m=video 40000 TCP/RTP/AVP 96 98\r\n"));
        assert!(sdp.contains("a=setup:passive\r\n"));
    }

    #[test]
    fn parses_answer_addr_and_ssrc() {
        let answer = "v=0\r\no=- 0 0 IN IP4 192.168.1.64\r\ns=Play\r\nc=IN IP4 192.168.1.64\r\nt=0 0\r\nm=video 15060 RTP/AVP 96\r\ny=0200000001\r\n";
        let a = parse_answer(answer).unwrap();
        assert_eq!(a.media_addr, "192.168.1.64:15060".parse().unwrap());
        assert_eq!(a.ssrc.as_deref(), Some("0200000001"));
    }
}
