//! rsipstack bootstrap shared by both facades: UDP bind, Endpoint + serve
//! task, DialogLayer, and the out-of-dialog MESSAGE send helper.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rsipstack::EndpointBuilder;
use rsipstack::dialog::dialog::{DialogStateReceiver, DialogStateSender};
use rsipstack::dialog::dialog_layer::DialogLayer;
use rsipstack::sip as rsip;
use rsipstack::transaction::TransactionReceiver;
use rsipstack::transaction::endpoint::{Endpoint, EndpointInnerRef};
use rsipstack::transaction::key::{TransactionKey, TransactionRole};
use rsipstack::transaction::make_tag;
use rsipstack::transaction::transaction::Transaction;
use rsipstack::transport::udp::UdpConnection;
use rsipstack::transport::{SipAddr, TransportLayer};
use tokio_util::sync::CancellationToken;

use crate::error::{GbError, Result};

pub(crate) fn sip_err(e: impl std::fmt::Display) -> GbError {
    GbError::Sip(e.to_string())
}

/// A bound SIP endpoint with its dialog layer and serve task running.
pub struct SipEndpoint {
    endpoint: Endpoint,
    pub dialog_layer: Arc<DialogLayer>,
    pub state_sender: DialogStateSender,
    pub cancel: CancellationToken,
    pub local_addr: SocketAddr,
    cseq: AtomicU32,
}

/// The two receive streams `bind_udp` hands back exactly once.
pub struct EndpointStreams {
    pub incoming: TransactionReceiver,
    pub state_receiver: DialogStateReceiver,
}

impl SipEndpoint {
    /// Bind a UDP SIP endpoint on `listen` (port 0 = ephemeral) and start
    /// its serve task. Returns the endpoint plus the receive streams.
    pub async fn bind_udp(listen: SocketAddr, user_agent: &str) -> Result<(Self, EndpointStreams)> {
        let cancel = CancellationToken::new();
        let transport_layer = TransportLayer::new(cancel.clone());
        let conn = UdpConnection::create_connection(listen, None, Some(cancel.child_token()))
            .await
            .map_err(sip_err)?;
        let local_addr = conn.get_addr().get_socketaddr().map_err(sip_err)?;
        transport_layer.add_transport(conn.into());

        let endpoint = EndpointBuilder::new()
            .with_user_agent(user_agent)
            .with_cancel_token(cancel.clone())
            .with_transport_layer(transport_layer)
            .build();
        let incoming = endpoint.incoming_transactions().map_err(sip_err)?;
        let dialog_layer = Arc::new(DialogLayer::new(endpoint.inner.clone()));
        let (state_sender, state_receiver) = dialog_layer.new_dialog_state_channel();

        let serve_inner = endpoint.inner.clone();
        tokio::spawn(async move {
            serve_inner.serve().await.ok();
        });

        Ok((
            Self {
                endpoint,
                dialog_layer,
                state_sender,
                cancel,
                local_addr,
                cseq: AtomicU32::new(1),
            },
            EndpointStreams {
                incoming,
                state_receiver,
            },
        ))
    }

    pub fn inner(&self) -> EndpointInnerRef {
        self.endpoint.inner.clone()
    }

    pub fn next_cseq(&self) -> u32 {
        self.cseq.fetch_add(1, Ordering::Relaxed)
    }

    /// Stop the serve task and all transports.
    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

/// Send one out-of-dialog MANSCDP MESSAGE to `dest` and wait for the final
/// response. `from`/`to` are `(user, domain)` pairs.
pub(crate) async fn send_out_of_dialog_message(
    endpoint: &EndpointInnerRef,
    from: (&str, &str),
    to: (&str, &str),
    dest: SipAddr,
    seq: u32,
    body: String,
) -> Result<rsip::Response> {
    let via = endpoint.get_via(None, None).map_err(sip_err)?;
    let from_uri: rsip::Uri = format!("sip:{}@{}", from.0, from.1)
        .try_into()
        .map_err(sip_err)?;
    let to_uri: rsip::Uri = format!("sip:{}@{}", to.0, to.1)
        .try_into()
        .map_err(sip_err)?;
    let from = rsip::typed::From {
        display_name: None,
        uri: from_uri,
        params: vec![],
    }
    .with_tag(make_tag());
    let to = rsip::typed::To {
        display_name: None,
        uri: to_uri.clone(),
        params: vec![],
    };
    let mut req = endpoint.make_request(rsip::Method::Message, to_uri, via, from, to, seq, None);
    req.body = body.into_bytes();
    req.headers
        .unique_push(rsip::Header::ContentType("Application/MANSCDP+xml".into()));
    req.headers
        .unique_push(rsip::Header::ContentLength((req.body.len() as u32).into()));
    let key = TransactionKey::from_request(&req, TransactionRole::Client).map_err(sip_err)?;
    let mut tx = Transaction::new_client(key, req, endpoint.clone(), None);
    tx.destination = Some(dest);
    tx.send().await.map_err(sip_err)?;
    while let Some(msg) = tx.receive().await {
        if let rsip::SipMessage::Response(resp) = msg {
            if matches!(resp.status_code.kind(), rsip::StatusCodeKind::Provisional) {
                continue;
            }
            return Ok(resp);
        }
    }
    Err(GbError::Timeout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Two endpoints on loopback: B replies 200 to any MESSAGE; A sends one
    /// and must get the 200 back. Proves bind/serve/tx plumbing end to end.
    #[tokio::test]
    async fn message_round_trip_between_two_endpoints() {
        let (a, _a_streams) = SipEndpoint::bind_udp("127.0.0.1:0".parse().unwrap(), "test-a")
            .await
            .unwrap();
        let (b, mut b_streams) = SipEndpoint::bind_udp("127.0.0.1:0".parse().unwrap(), "test-b")
            .await
            .unwrap();

        let responder = tokio::spawn(async move {
            if let Some(mut tx) = b_streams.incoming.recv().await {
                let body = String::from_utf8_lossy(&tx.original.body).to_string();
                tx.reply(rsip::StatusCode::OK).await.unwrap();
                return body;
            }
            String::new()
        });

        let dest = SipAddr::from(b.local_addr);
        let resp = tokio::time::timeout(
            Duration::from_secs(5),
            send_out_of_dialog_message(
                &a.inner(),
                ("34020000002000000001", "3402000000"),
                ("34020000001320000001", "3402000000"),
                dest,
                a.next_cseq(),
                "<hello/>".to_string(),
            ),
        )
        .await
        .expect("send timed out")
        .expect("send failed");
        assert_eq!(resp.status_code, rsip::StatusCode::OK);

        let received = tokio::time::timeout(Duration::from_secs(5), responder)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, "<hello/>");

        a.shutdown();
        b.shutdown();
    }
}
