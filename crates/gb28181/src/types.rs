//! Shared pure data types (no networking).

use std::net::SocketAddr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Udp,
    TcpPassive,
    TcpActive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamType {
    Play,
    Playback { start: i64, end: i64 }, // unix seconds
    Download,
}

/// The media handoff contract (spec §5.1). Pure data; the crate never touches RTP.
#[derive(Debug, Clone)]
pub struct MediaSpec {
    pub ssrc: u32,
    pub ssrc_str: String,
    pub transport: Transport,
    /// For receive (server): our local receive addr. For send (client): the remote target.
    pub media_addr: SocketAddr,
    pub stream_type: StreamType,
    /// Negotiated remote media addr, set after 200 OK (required for TcpActive connect).
    pub negotiated_remote: Option<SocketAddr>,
}

/// A device currently in the registrar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredDevice {
    pub device_id: String,
    pub contact: String,
    pub transport: Transport,
    pub expires_at: i64,     // unix seconds
    pub last_keepalive: i64, // unix seconds
    pub online: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_spec_constructs() {
        let spec = MediaSpec {
            ssrc: 200000001,
            ssrc_str: "0200000001".into(),
            transport: Transport::Udp,
            media_addr: "0.0.0.0:30000".parse().unwrap(),
            stream_type: StreamType::Play,
            negotiated_remote: None,
        };
        assert_eq!(spec.transport, Transport::Udp);
        assert_eq!(spec.ssrc_str, "0200000001");
    }
}
