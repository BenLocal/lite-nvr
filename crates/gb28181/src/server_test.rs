use std::time::Duration;

use rsipstack::dialog::{authenticate::Credential, registration::Registration};
use rsipstack::sip as rsip;
use tokio::sync::mpsc::UnboundedReceiver;

use super::*;
use crate::event::GbEvent;

pub(crate) async fn wait_for(
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

/// A raw rsipstack UAC that REGISTERs to `server_addr`. The From/To user is
/// always credential.username — pass one even for Open-auth servers.
pub(crate) async fn raw_register(
    device_id: &str,
    password: &str,
    domain: &str,
    server_id: &str,
    server_addr: std::net::SocketAddr,
    expires: u32,
) -> rsip::Response {
    let (ep, _streams) =
        crate::endpoint::SipEndpoint::bind_udp("127.0.0.1:0".parse().unwrap(), "raw-test-client")
            .await
            .unwrap();
    let cred = Credential {
        username: device_id.to_string(),
        password: password.to_string(),
        realm: Some(domain.to_string()),
    };
    let mut reg = Registration::new(ep.inner(), Some(cred));
    let uri: rsip::Uri = format!("sip:{}@{}", server_id, server_addr)
        .try_into()
        .unwrap();
    let resp = tokio::time::timeout(Duration::from_secs(5), reg.register(uri, Some(expires)))
        .await
        .expect("register timed out")
        .expect("register transaction failed");
    ep.shutdown();
    resp
}

const PLATFORM: &str = "34020000002000000001";
const DOMAIN: &str = "3402000000";
const DEVICE: &str = "34020000001320000001";

#[tokio::test]
async fn open_auth_register_emits_registered_and_lists_device() {
    let cfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    let (server, mut events) = GbServer::bind(cfg).await.unwrap();

    let resp = raw_register(
        DEVICE,
        "ignored",
        DOMAIN,
        PLATFORM,
        server.local_addr(),
        3600,
    )
    .await;
    assert_eq!(resp.status_code, rsip::StatusCode::OK);

    let e = wait_for(&mut events, |e| matches!(e, GbEvent::Registered { .. })).await;
    assert!(matches!(e, GbEvent::Registered { device_id } if device_id == DEVICE));

    let devices = server.devices();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].device_id, DEVICE);
    assert!(devices[0].online);
    server.shutdown();
}

#[tokio::test]
async fn expires_zero_unregisters() {
    let cfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    let (server, mut events) = GbServer::bind(cfg).await.unwrap();

    raw_register(DEVICE, "x", DOMAIN, PLATFORM, server.local_addr(), 3600).await;
    wait_for(&mut events, |e| matches!(e, GbEvent::Registered { .. })).await;

    let resp = raw_register(DEVICE, "x", DOMAIN, PLATFORM, server.local_addr(), 0).await;
    assert_eq!(resp.status_code, rsip::StatusCode::OK);
    wait_for(&mut events, |e| matches!(e, GbEvent::Unregistered { .. })).await;
    assert!(server.devices().is_empty());
    server.shutdown();
}

#[tokio::test]
async fn shared_password_register_succeeds_via_digest() {
    let mut cfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    cfg.auth = crate::auth::AuthConfig::Shared("s3cret".into());
    let (server, mut events) = GbServer::bind(cfg).await.unwrap();

    // rsipstack answers the 401 challenge internally.
    let resp = raw_register(
        DEVICE,
        "s3cret",
        DOMAIN,
        PLATFORM,
        server.local_addr(),
        3600,
    )
    .await;
    assert_eq!(resp.status_code, rsip::StatusCode::OK);
    wait_for(&mut events, |e| matches!(e, GbEvent::Registered { .. })).await;
    server.shutdown();
}

#[tokio::test]
async fn wrong_password_is_rejected() {
    let mut cfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    cfg.auth = crate::auth::AuthConfig::Shared("s3cret".into());
    let (server, _events) = GbServer::bind(cfg).await.unwrap();

    // The client answers the challenge with a bad digest; the server
    // re-challenges; rsipstack gives up after one auth attempt and returns
    // the final 401.
    let resp = raw_register(DEVICE, "WRONG", DOMAIN, PLATFORM, server.local_addr(), 3600).await;
    assert_ne!(resp.status_code, rsip::StatusCode::OK);
    assert!(server.devices().is_empty());
    server.shutdown();
}

#[tokio::test]
async fn provider_unknown_device_is_forbidden() {
    let mut cfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    cfg.auth = crate::auth::AuthConfig::Provider(Box::new(|id| {
        (id == "known-device").then(|| "pw".to_string())
    }));
    let (server, _events) = GbServer::bind(cfg).await.unwrap();

    let resp = raw_register(DEVICE, "pw", DOMAIN, PLATFORM, server.local_addr(), 3600).await;
    assert_eq!(resp.status_code, rsip::StatusCode::Forbidden);
    assert!(server.devices().is_empty());
    server.shutdown();
}

/// Send one MANSCDP body as a MESSAGE from a throwaway endpoint.
pub(crate) async fn raw_message(
    server_addr: std::net::SocketAddr,
    from_id: &str,
    to_id: &str,
    body: String,
) -> rsip::Response {
    let (ep, _streams) =
        crate::endpoint::SipEndpoint::bind_udp("127.0.0.1:0".parse().unwrap(), "raw-test-client")
            .await
            .unwrap();
    let dest = rsipstack::transport::SipAddr::from(server_addr);
    let resp = tokio::time::timeout(
        Duration::from_secs(5),
        crate::endpoint::send_out_of_dialog_message(
            &ep.inner(),
            (from_id, DOMAIN),
            (to_id, DOMAIN),
            dest,
            ep.next_cseq(),
            body,
        ),
    )
    .await
    .expect("message timed out")
    .expect("message failed");
    ep.shutdown();
    resp
}

#[tokio::test]
async fn keepalive_refreshes_and_emits_event() {
    let cfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    let (server, mut events) = GbServer::bind(cfg).await.unwrap();
    raw_register(DEVICE, "x", DOMAIN, PLATFORM, server.local_addr(), 3600).await;
    wait_for(&mut events, |e| matches!(e, GbEvent::Registered { .. })).await;

    let body = crate::manscdp::encode_keepalive_notify(1, DEVICE);
    let resp = raw_message(server.local_addr(), DEVICE, PLATFORM, body).await;
    assert_eq!(resp.status_code, rsip::StatusCode::OK);
    wait_for(&mut events, |e| {
        matches!(e, GbEvent::KeepaliveReceived { .. })
    })
    .await;
    server.shutdown();
}

#[tokio::test]
async fn keepalive_from_unknown_device_is_not_found() {
    let cfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    let (server, _events) = GbServer::bind(cfg).await.unwrap();
    let body = crate::manscdp::encode_keepalive_notify(1, "99999999999999999999");
    let resp = raw_message(server.local_addr(), "99999999999999999999", PLATFORM, body).await;
    assert_eq!(resp.status_code, rsip::StatusCode::NotFound);
    server.shutdown();
}

#[tokio::test]
async fn missed_keepalive_marks_offline() {
    let mut cfg = GbServerConfig::new(PLATFORM, DOMAIN, "127.0.0.1:0".parse().unwrap());
    cfg.keepalive_grace = 1; // offline after 1s without keepalive
    cfg.sweep_interval = Duration::from_millis(100);
    let (server, mut events) = GbServer::bind(cfg).await.unwrap();
    raw_register(DEVICE, "x", DOMAIN, PLATFORM, server.local_addr(), 3600).await;
    wait_for(&mut events, |e| matches!(e, GbEvent::Registered { .. })).await;

    // No keepalive for >1s -> sweep flips it offline (registration not expired).
    let e = wait_for(&mut events, |e| matches!(e, GbEvent::Offline { .. })).await;
    assert!(matches!(e, GbEvent::Offline { device_id } if device_id == DEVICE));
    let devices = server.devices();
    assert_eq!(devices.len(), 1);
    assert!(!devices[0].online);
    server.shutdown();
}
