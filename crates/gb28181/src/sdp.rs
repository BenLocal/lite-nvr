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
    // TODO(P1-3/real-device): emit the GB/T 28181 Annex B `f=` (and `u=`) media-format line once validated against a real device
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

// --- P1-2: the answerer's view (client role) ---

/// Parsed fields from an INVITE offer, from the OFFERER's perspective:
/// `media_addr` is where the offerer wants to receive media; `transport`
/// is what the offerer advertised (`a=setup:active` = offerer connects out).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OfferSdp {
    pub session: String, // "Play" | "Playback" | "Download" (as sent)
    pub media_addr: SocketAddr,
    pub ssrc: Option<String>,
    pub transport: Transport,
    /// Playback/Download time window from the `t=` line (Unix seconds); `0 0`
    /// for live Play.
    pub start: u64,
    pub stop: u64,
}

/// Parse `s=`, `c=`, `m=video`, `y=` and `a=setup:` from an offer.
pub fn parse_offer(sdp: &str) -> Result<OfferSdp> {
    let mut session = String::new();
    let mut ip: Option<&str> = None;
    let mut port: Option<u16> = None;
    let mut ssrc: Option<String> = None;
    let mut tcp = false;
    let mut setup_active = false;
    let mut start: u64 = 0;
    let mut stop: u64 = 0;
    for line in sdp.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("s=") {
            session = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("t=") {
            let mut parts = rest.split_whitespace();
            start = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
            stop = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("c=IN IP4 ") {
            ip = Some(rest.trim());
        } else if let Some(rest) = line.strip_prefix("m=video ") {
            let mut parts = rest.split_whitespace();
            port = parts.next().and_then(|p| p.parse().ok());
            tcp = parts
                .next()
                .map(|proto| proto.contains("TCP"))
                .unwrap_or(false);
        } else if let Some(rest) = line.strip_prefix("y=") {
            ssrc = Some(rest.trim().to_string());
        } else if line == "a=setup:active" {
            setup_active = true;
        }
    }
    let ip = ip.ok_or_else(|| GbError::Sdp("missing c= line".into()))?;
    let port = port.ok_or_else(|| GbError::Sdp("missing m=video port".into()))?;
    let media_addr: SocketAddr = format!("{ip}:{port}")
        .parse()
        .map_err(|e| GbError::Sdp(format!("bad addr {ip}:{port}: {e}")))?;
    let transport = if !tcp {
        Transport::Udp
    } else if setup_active {
        Transport::TcpActive
    } else {
        Transport::TcpPassive
    };
    Ok(OfferSdp {
        session,
        media_addr,
        ssrc,
        transport,
        start,
        stop,
    })
}

/// Build the answer SDP a device sends back accepting a Play INVITE.
/// `media_ip:media_port` is where the device will SEND media FROM (its RTP
/// source); the `y=` line echoes the offer's SSRC.
pub fn build_answer(
    local_id: &str,
    media_ip: &str,
    media_port: u16,
    ssrc_str: &str,
    transport: Transport,
    session: &str,
    start: u64,
    stop: u64,
) -> String {
    let proto = match transport {
        Transport::Udp => "RTP/AVP",
        Transport::TcpPassive | Transport::TcpActive => "TCP/RTP/AVP",
    };
    let mut sdp = String::new();
    sdp.push_str("v=0\r\n");
    sdp.push_str(&format!("o={local_id} 0 0 IN IP4 {media_ip}\r\n"));
    sdp.push_str(&format!("s={session}\r\n"));
    sdp.push_str(&format!("c=IN IP4 {media_ip}\r\n"));
    sdp.push_str(&format!("t={start} {stop}\r\n"));
    sdp.push_str(&format!("m=video {media_port} {proto} 96\r\n"));
    sdp.push_str("a=sendonly\r\n");
    sdp.push_str("a=rtpmap:96 PS/90000\r\n");
    sdp.push_str(&format!("y={ssrc_str}\r\n"));
    sdp
}

#[cfg(test)]
mod p1_2_sdp_tests {
    use super::*;
    use crate::types::StreamType;

    #[test]
    fn offer_round_trips_through_parse_offer() {
        let offer = build_play_offer(
            "34020000002000000001",
            "127.0.0.1",
            30000,
            "0200000001",
            Transport::Udp,
            &StreamType::Play,
        );
        let o = parse_offer(&offer).unwrap();
        assert_eq!(o.session, "Play");
        assert_eq!(o.media_addr, "127.0.0.1:30000".parse().unwrap());
        assert_eq!(o.ssrc.as_deref(), Some("0200000001"));
        assert_eq!(o.transport, Transport::Udp);
    }

    #[test]
    fn parse_offer_detects_tcp_variants() {
        for (t, expect) in [
            (Transport::TcpPassive, Transport::TcpPassive),
            (Transport::TcpActive, Transport::TcpActive),
        ] {
            let offer =
                build_play_offer("id", "10.0.0.1", 40000, "0200000002", t, &StreamType::Play);
            assert_eq!(parse_offer(&offer).unwrap().transport, expect);
        }
    }

    #[test]
    fn answer_parses_with_existing_parse_answer() {
        let ans = build_answer(
            "34020000001320000001",
            "127.0.0.1",
            40002,
            "0200000001",
            Transport::Udp,
            "Play",
            0,
            0,
        );
        let a = parse_answer(&ans).unwrap();
        assert_eq!(a.media_addr, "127.0.0.1:40002".parse().unwrap());
        assert_eq!(a.ssrc.as_deref(), Some("0200000001"));
        assert!(ans.contains("a=sendonly\r\n"));
        assert!(ans.contains("s=Play\r\n"));
    }

    #[test]
    fn parse_offer_reads_playback_time_range() {
        let offer = "v=0\r\no=34020000001320000001 0 0 IN IP4 127.0.0.1\r\n\
s=Playback\r\nc=IN IP4 127.0.0.1\r\nt=1704067200 1704070800\r\n\
m=video 40000 RTP/AVP 96\r\ny=0200000001\r\n";
        let o = parse_offer(offer).unwrap();
        assert_eq!(o.session, "Playback");
        assert_eq!(o.start, 1704067200);
        assert_eq!(o.stop, 1704070800);
        // A Playback answer echoes the session + range.
        let ans = build_answer(
            "dev",
            "127.0.0.1",
            40002,
            "0200000001",
            Transport::Udp,
            &o.session,
            o.start,
            o.stop,
        );
        assert!(ans.contains("s=Playback\r\n"));
        assert!(ans.contains("t=1704067200 1704070800\r\n"));
    }
}
