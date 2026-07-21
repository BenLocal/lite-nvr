use std::net::{Ipv4Addr, SocketAddr};

use quick_xml::Reader;
use quick_xml::events::Event;
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use uuid::Uuid;

use crate::config::DeviceCfg;

const WS_DISCOVERY_ADDR: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const WS_DISCOVERY_PORT: u16 = 3702;

/// Pull `wsa:MessageID` out of a Probe so we can echo it as RelatesTo.
pub fn extract_message_id(probe: &str) -> Option<String> {
    let mut reader = Reader::from_str(probe);
    let mut buf = Vec::new();
    let mut in_id = false;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                in_id = e.local_name().as_ref() == b"MessageID";
            }
            Ok(Event::Text(t)) if in_id => {
                return Some(t.unescape().unwrap_or_default().to_string());
            }
            Ok(Event::End(_)) => in_id = false,
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

/// A ProbeMatches reply. Scopes carry the device name + hardware, which the
/// onvif-rs client surfaces as `Device.name` / `Device.hardware`.
pub fn probe_matches_xml(
    msg_id: &str,
    relates_to: &str,
    xaddr: &str,
    name: &str,
    hardware: &str,
) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope"
 xmlns:w="http://schemas.xmlsoap.org/ws/2004/08/addressing"
 xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery"
 xmlns:dn="http://www.onvif.org/ver10/network/wsdl">
<e:Header>
<w:MessageID>urn:uuid:{msg_id}</w:MessageID>
<w:RelatesTo>{relates_to}</w:RelatesTo>
<w:To>http://schemas.xmlsoap.org/ws/2004/08/addressing/role/anonymous</w:To>
<w:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/ProbeMatches</w:Action>
</e:Header>
<e:Body><d:ProbeMatches><d:ProbeMatch>
<w:EndpointReference><w:Address>urn:uuid:{msg_id}</w:Address></w:EndpointReference>
<d:Types>dn:NetworkVideoTransmitter</d:Types>
<d:Scopes>onvif://www.onvif.org/type/video_encoder onvif://www.onvif.org/name/{name} onvif://www.onvif.org/hardware/{hardware} onvif://www.onvif.org/location/dummy</d:Scopes>
<d:XAddrs>{xaddr}</d:XAddrs>
<d:MetadataVersion>1</d:MetadataVersion>
</d:ProbeMatch></d:ProbeMatches></e:Body></e:Envelope>"#
    )
}

/// Bind 3702, join the multicast group, and answer every Probe with a unicast
/// ProbeMatches. Runs until the task is aborted.
pub async fn run(cfg: DeviceCfg) -> anyhow::Result<()> {
    // Build a reuse-addr UDP socket bound to 0.0.0.0:3702 and join the group.
    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    sock.set_reuse_address(true)?;
    let bind: SocketAddr = format!("0.0.0.0:{WS_DISCOVERY_PORT}").parse()?;
    sock.bind(&bind.into())?;
    sock.join_multicast_v4(&WS_DISCOVERY_ADDR, &Ipv4Addr::UNSPECIFIED)?;
    sock.set_nonblocking(true)?;
    let udp = UdpSocket::from_std(sock.into())?;

    let xaddr = cfg.service_url();
    let mut buf = vec![0u8; 64 * 1024];
    log::info!("ws-discovery: listening on 239.255.255.250:{WS_DISCOVERY_PORT}");
    loop {
        let (n, from) = match udp.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("ws-discovery recv: {e}");
                continue;
            }
        };
        let probe = String::from_utf8_lossy(&buf[..n]);
        if !probe.contains("Probe") {
            continue;
        }
        let relates_to = extract_message_id(&probe).unwrap_or_default();
        let reply = probe_matches_xml(
            &Uuid::new_v4().to_string(),
            &relates_to,
            &xaddr,
            &cfg.model,
            &cfg.manufacturer,
        );
        if let Err(e) = udp.send_to(reply.as_bytes(), from).await {
            log::warn!("ws-discovery send: {e}");
        } else {
            log::info!("ws-discovery: answered probe from {from}");
        }
    }
}

#[cfg(test)]
#[path = "discovery_test.rs"]
mod discovery_test;
