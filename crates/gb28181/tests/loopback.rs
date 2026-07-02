//! client↔server end-to-end signaling over UDP loopback:
//! REGISTER (digest) → Keepalive → Catalog → INVITE Play → BYE.
//! No RTP flows — both sides only exchange MediaSpec data (spec §3 row 10).

use std::time::Duration;

use gb28181::{
    AuthConfig, CatalogItem, GbClient, GbClientConfig, GbEvent, GbServer, GbServerConfig,
    MediaSpec, SsrcKind, StreamType, Transport,
};
use tokio::sync::mpsc::UnboundedReceiver;

const PLATFORM: &str = "34020000002000000001";
const DOMAIN: &str = "3402000000";
const DEVICE: &str = "34020000001110000001";
const CHANNEL: &str = "34020000001320000001";
const PASSWORD: &str = "loopback-pw";

async fn wait_for(
    rx: &mut UnboundedReceiver<GbEvent>,
    mut pred: impl FnMut(&GbEvent) -> bool,
) -> GbEvent {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let e = rx.recv().await.expect("event channel closed");
            if pred(&e) {
                return e;
            }
        }
    })
    .await
    .expect("timed out waiting for event")
}

#[tokio::test]
async fn full_signaling_loopback() {
    // -- server (上级平台) --
    let mut scfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    scfg.auth = AuthConfig::Shared(PASSWORD.into());
    let (server, mut server_events) = GbServer::bind(scfg).await.unwrap();

    // -- client (下级设备) --
    let mut ccfg = GbClientConfig::new(DEVICE, DOMAIN, PLATFORM, server.local_addr());
    ccfg.listen = "127.0.0.1:0".parse().unwrap();
    ccfg.password = Some(PASSWORD.into());
    ccfg.keepalive_interval = Duration::from_millis(200);
    ccfg.channels = vec![CatalogItem {
        device_id: CHANNEL.into(),
        name: "回环测试通道".into(),
        status: "ON".into(),
    }];
    let (client, mut client_events) = GbClient::bind(ccfg).await.unwrap();

    // 1. REGISTER with digest auth
    client.register().await.unwrap();
    wait_for(
        &mut server_events,
        |e| matches!(e, GbEvent::Registered { device_id } if device_id == DEVICE),
    )
    .await;
    assert_eq!(server.devices().len(), 1);

    // 2. Keepalive flows
    wait_for(
        &mut server_events,
        |e| matches!(e, GbEvent::KeepaliveReceived { device_id } if device_id == DEVICE),
    )
    .await;

    // 3. Catalog
    let catalog = server.catalog_query(DEVICE).await.unwrap();
    assert!(!catalog.incomplete);
    assert_eq!(catalog.items.len(), 1);
    assert_eq!(catalog.items[0].device_id, CHANNEL);
    assert_eq!(catalog.items[0].name, "回环测试通道");

    // 4. INVITE Play — both sides must end with matching MediaSpecs
    let (ssrc, ssrc_str) = server.next_ssrc(SsrcKind::Live);
    let spec = MediaSpec {
        ssrc,
        ssrc_str: ssrc_str.clone(),
        transport: Transport::Udp,
        media_addr: "127.0.0.1:31000".parse().unwrap(),
        stream_type: StreamType::Play,
        negotiated_remote: None,
    };
    let answerer = tokio::spawn(async move {
        loop {
            let e = client_events.recv().await.expect("client events closed");
            if let GbEvent::InviteReceived(negotiation) = e {
                assert_eq!(
                    negotiation.remote.media_addr,
                    "127.0.0.1:31000".parse().unwrap()
                );
                assert_eq!(negotiation.remote.ssrc.as_deref(), Some(ssrc_str.as_str()));
                let handle = negotiation
                    .answer("127.0.0.1:41000".parse().unwrap())
                    .unwrap();
                return (handle, client_events);
            }
        }
    });
    let session = server.invite_play(DEVICE, CHANNEL, spec).await.unwrap();
    assert_eq!(
        session.spec.negotiated_remote,
        Some("127.0.0.1:41000".parse().unwrap())
    );
    let (_handle, mut client_events) = tokio::time::timeout(Duration::from_secs(5), answerer)
        .await
        .unwrap()
        .unwrap();

    // 5. BYE from the platform side
    session.stop().await.unwrap();
    wait_for(&mut client_events, |e| {
        matches!(e, GbEvent::SessionClosed { .. })
    })
    .await;

    client.shutdown();
    server.shutdown();
}
