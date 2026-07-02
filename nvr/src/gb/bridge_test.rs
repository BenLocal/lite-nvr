use std::time::Duration;

use gb28181::{CatalogItem, GbClient, GbClientConfig, GbEvent, GbServer, GbServerConfig};

use super::*;
use crate::gb::receiver::fake::FakeReceiver;

const PLATFORM: &str = "34020000002000000001";
const DOMAIN: &str = "3402000000";
const DEVICE: &str = "34020000001110000001";
const CHANNEL: &str = "34020000001320000001";

async fn wait_registered(rx: &mut tokio::sync::mpsc::UnboundedReceiver<GbEvent>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(e) = rx.recv().await {
            if matches!(e, GbEvent::Registered { .. }) {
                return;
            }
        }
    })
    .await
    .expect("device never registered");
}

/// Spawn a GbClient that registers to `server_addr` and auto-answers INVITEs
/// pointing its media at a throwaway addr (we only assert signaling here).
async fn spawn_answering_client(
    server_addr: std::net::SocketAddr,
) -> (GbClient, tokio::task::JoinHandle<()>) {
    let mut ccfg = GbClientConfig::new(DEVICE, DOMAIN, PLATFORM, server_addr);
    ccfg.listen = "127.0.0.1:0".parse().unwrap();
    ccfg.channels = vec![CatalogItem {
        device_id: CHANNEL.into(),
        name: "cam".into(),
        status: "ON".into(),
    }];
    let (client, mut events) = GbClient::bind(ccfg).await.unwrap();
    client.register().await.unwrap();
    let answerer = tokio::spawn(async move {
        while let Some(e) = events.recv().await {
            if let GbEvent::InviteReceived(neg) = e {
                neg.answer("127.0.0.1:40010".parse().unwrap()).ok();
            }
        }
    });
    (client, answerer)
}

#[tokio::test]
async fn pull_on_not_found_then_release_on_no_reader() {
    // platform (bridge's GbServer)
    let scfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    let (server, mut server_events) = GbServer::bind(scfg).await.unwrap();
    let server_addr = server.local_addr();

    let bridge = GbBridge::new(
        server,
        "127.0.0.1".into(),
        Box::new(FakeReceiver::default()),
    );

    // a lower device registers + answers
    let (client, answerer) = spawn_answering_client(server_addr).await;
    wait_registered(&mut server_events).await;

    // map the ZLM stream "cam1" -> this device/channel
    bridge.register_mapping("cam1", DEVICE, CHANNEL);

    // unknown stream is ignored
    assert!(!bridge.handle_media_not_found("nope").await);
    assert!(!bridge.is_active("nope"));

    // known stream: pull starts, session becomes active
    assert!(bridge.handle_media_not_found("cam1").await);
    assert!(bridge.is_active("cam1"));

    // idempotent: a second not_found doesn't double-pull
    assert!(bridge.handle_media_not_found("cam1").await);
    assert!(bridge.is_active("cam1"));

    // no reader: session released
    bridge.handle_media_no_reader("cam1").await;
    assert!(!bridge.is_active("cam1"));

    // unregister also tears down (register + pull again, then unregister)
    bridge.register_mapping("cam1", DEVICE, CHANNEL);
    assert!(bridge.handle_media_not_found("cam1").await);
    assert!(bridge.is_active("cam1"));
    bridge.unregister_mapping("cam1").await;
    assert!(!bridge.is_active("cam1"));
    assert!(!bridge.handle_media_not_found("cam1").await); // mapping gone

    client.shutdown();
    answerer.abort();
}
