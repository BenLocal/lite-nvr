use std::time::Duration;

use super::*;
use crate::event::GbEvent;
use crate::server::server_test::wait_for;
use crate::server::{GbServer, GbServerConfig};

const PLATFORM: &str = "34020000002000000001";
const DOMAIN: &str = "3402000000";
const DEVICE: &str = "34020000001110000001";

pub(crate) fn test_channels() -> Vec<CatalogItem> {
    vec![CatalogItem {
        device_id: "34020000001320000001".into(),
        name: "door".into(),
        status: "ON".into(),
    }]
}

pub(crate) async fn bound_pair(
    password: Option<&str>,
) -> (
    GbServer,
    tokio::sync::mpsc::UnboundedReceiver<GbEvent>,
    GbClient,
    tokio::sync::mpsc::UnboundedReceiver<GbEvent>,
) {
    let mut scfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    if let Some(pw) = password {
        scfg.auth = crate::auth::AuthConfig::Shared(pw.into());
    }
    let (server, server_events) = GbServer::bind(scfg).await.unwrap();

    let mut ccfg = GbClientConfig::new(DEVICE, DOMAIN, PLATFORM, server.local_addr());
    ccfg.listen = "127.0.0.1:0".parse().unwrap();
    ccfg.password = password.map(|s| s.to_string());
    ccfg.channels = test_channels();
    ccfg.keepalive_interval = Duration::from_millis(200);
    let (client, client_events) = GbClient::bind(ccfg).await.unwrap();

    (server, server_events, client, client_events)
}

#[tokio::test]
async fn client_registers_with_digest_and_keeps_alive() {
    let (server, mut server_events, client, _client_events) = bound_pair(Some("s3cret")).await;

    client.register().await.unwrap();
    wait_for(
        &mut server_events,
        |e| matches!(e, GbEvent::Registered { device_id } if device_id == DEVICE),
    )
    .await;

    // keepalive_interval = 200ms -> a KeepaliveReceived within the 5s window.
    wait_for(
        &mut server_events,
        |e| matches!(e, GbEvent::KeepaliveReceived { device_id } if device_id == DEVICE),
    )
    .await;

    let devices = server.devices();
    assert_eq!(devices.len(), 1);
    assert!(devices[0].online);

    client.shutdown();
    server.shutdown();
}

#[tokio::test]
async fn client_register_fails_with_wrong_password() {
    let (server, _server_events, client, _client_events) = bound_pair(Some("s3cret")).await;
    // Rebind a client with the wrong password against the same server.
    let mut ccfg = GbClientConfig::new(DEVICE, DOMAIN, PLATFORM, server.local_addr());
    ccfg.listen = "127.0.0.1:0".parse().unwrap();
    ccfg.password = Some("WRONG".into());
    let (bad_client, _ev) = GbClient::bind(ccfg).await.unwrap();

    let err = tokio::time::timeout(Duration::from_secs(5), bad_client.register())
        .await
        .expect("register timed out");
    assert!(err.is_err());

    bad_client.shutdown();
    client.shutdown();
    server.shutdown();
}

#[tokio::test]
async fn catalog_query_returns_client_channels() {
    let (server, mut server_events, client, _client_events) = bound_pair(None).await;
    client.register().await.unwrap();
    wait_for(&mut server_events, |e| {
        matches!(e, GbEvent::Registered { .. })
    })
    .await;

    let catalog = tokio::time::timeout(Duration::from_secs(10), server.catalog_query(DEVICE))
        .await
        .expect("catalog query timed out")
        .expect("catalog query failed");
    assert!(!catalog.incomplete);
    assert_eq!(catalog.items, test_channels());

    client.shutdown();
    server.shutdown();
}

#[tokio::test]
async fn catalog_query_for_unknown_device_is_offline_error() {
    let (server, _se, client, _ce) = bound_pair(None).await;
    let err = server.catalog_query("99999999999999999999").await;
    assert!(matches!(err, Err(crate::error::GbError::DeviceOffline(_))));
    client.shutdown();
    server.shutdown();
}
