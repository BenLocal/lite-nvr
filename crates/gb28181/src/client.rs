//! GbClient — the UAC / 下级设备 facade. Registers up to a platform, answers
//! Catalog queries and INVITEs. Media is the caller's job (spec §3 row 7).

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rsipstack::dialog::authenticate::Credential;
use rsipstack::dialog::dialog::{Dialog, DialogState};
use rsipstack::dialog::registration::Registration;
use rsipstack::dialog::server_dialog::ServerInviteDialog;
use rsipstack::sip as rsip;
use rsipstack::sip::prelude::HeadersExt;
use rsipstack::transaction::TransactionReceiver;
use rsipstack::transaction::transaction::Transaction;
use rsipstack::transport::SipAddr;
use tokio::sync::mpsc;

use crate::endpoint::{SipEndpoint, send_out_of_dialog_message, sip_err};
use crate::error::{GbError, Result};
use crate::event::GbEvent;
use crate::manscdp::{self, CatalogItem, CmdType, RecordItem};
use crate::sdp;

pub struct GbClientConfig {
    /// Our 20-digit device GB code.
    pub device_id: String,
    /// SIP domain / digest realm shared with the platform.
    pub domain: String,
    /// The platform's GB code (Request-URI user for REGISTER/MESSAGE).
    pub server_id: String,
    /// The platform's SIP UDP address.
    pub server_addr: SocketAddr,
    pub password: Option<String>,
    /// Local UDP listen address (port 0 = ephemeral).
    pub listen: SocketAddr,
    /// Channels reported in Catalog responses.
    pub channels: Vec<CatalogItem>,
    pub expires: u32,
    pub keepalive_interval: Duration,
    pub user_agent: String,
    /// Device metadata reported in DeviceInfo responses.
    pub device_name: String,
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    /// Recordings advertised in RecordInfo responses (设备录像文件).
    pub records: Vec<RecordItem>,
}

impl GbClientConfig {
    pub fn new(
        device_id: impl Into<String>,
        domain: impl Into<String>,
        server_id: impl Into<String>,
        server_addr: SocketAddr,
    ) -> Self {
        Self {
            device_id: device_id.into(),
            domain: domain.into(),
            server_id: server_id.into(),
            server_addr,
            password: None,
            listen: "0.0.0.0:5061".parse().expect("static addr"),
            channels: Vec::new(),
            expires: 3600,
            keepalive_interval: Duration::from_secs(60),
            user_agent: "lite-nvr-gb28181-client".into(),
            device_name: "lite-nvr device".into(),
            manufacturer: "lite-nvr".into(),
            model: "gb28181-client".into(),
            firmware: "0.1".into(),
            records: Vec::new(),
        }
    }
}

struct ClientInner {
    cfg: GbClientConfig,
    ep: SipEndpoint,
    events: mpsc::UnboundedSender<GbEvent>,
    sn: AtomicU64,
    keepalive_running: AtomicBool,
    /// Dialog ids for INVITEs we actually answered (accepted). Only these get
    /// a `SessionClosed` when they terminate; rejected INVITEs stay silent.
    accepted: Arc<Mutex<HashSet<String>>>,
}

pub struct GbClient {
    inner: Arc<ClientInner>,
}

impl GbClient {
    pub async fn bind(cfg: GbClientConfig) -> Result<(GbClient, mpsc::UnboundedReceiver<GbEvent>)> {
        let (ep, streams) = SipEndpoint::bind_udp(cfg.listen, &cfg.user_agent).await?;
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(ClientInner {
            cfg,
            ep,
            events: events_tx,
            sn: AtomicU64::new(1),
            keepalive_running: AtomicBool::new(false),
            accepted: Arc::new(Mutex::new(HashSet::new())),
        });
        tokio::spawn(main_loop(inner.clone(), streams.incoming));
        tokio::spawn(state_loop(inner.clone(), streams.state_receiver));
        Ok((GbClient { inner }, events_rx))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.inner.ep.local_addr
    }

    /// REGISTER to the platform (rsipstack auto-answers the digest challenge)
    /// and start the keepalive loop on success.
    pub async fn register(&self) -> Result<()> {
        self.send_register(self.inner.cfg.expires, "register")
            .await?;
        self.start_keepalive();
        Ok(())
    }

    /// Unregister from the platform: REGISTER with `Expires: 0` (设备注销).
    /// Best-effort; call before dropping the client on shutdown.
    pub async fn unregister(&self) -> Result<()> {
        self.send_register(0, "unregister").await
    }

    async fn send_register(&self, expires: u32, what: &str) -> Result<()> {
        let cfg = &self.inner.cfg;
        let credential = Credential {
            username: cfg.device_id.clone(),
            password: cfg.password.clone().unwrap_or_default(),
            realm: Some(cfg.domain.clone()),
        };
        let mut reg = Registration::new(self.inner.ep.inner(), Some(credential));
        let uri: rsip::Uri = format!("sip:{}@{}", cfg.server_id, cfg.server_addr)
            .try_into()
            .map_err(sip_err)?;
        let resp = reg.register(uri, Some(expires)).await.map_err(sip_err)?;
        if resp.status_code != rsip::StatusCode::OK {
            return Err(GbError::Auth(format!(
                "{what} rejected: {}",
                resp.status_code
            )));
        }
        Ok(())
    }

    fn start_keepalive(&self) {
        if self.inner.keepalive_running.swap(true, Ordering::SeqCst) {
            return; // already running
        }
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(inner.cfg.keepalive_interval);
            tick.tick().await; // first tick fires immediately; skip it
            loop {
                tokio::select! {
                    _ = inner.ep.cancel.cancelled() => return,
                    _ = tick.tick() => {}
                }
                let sn = inner.sn.fetch_add(1, Ordering::Relaxed);
                let body = manscdp::encode_keepalive_notify(sn, &inner.cfg.device_id);
                let dest = SipAddr::from(inner.cfg.server_addr);
                if let Err(e) = send_out_of_dialog_message(
                    &inner.ep.inner(),
                    (&inner.cfg.device_id, &inner.cfg.domain),
                    (&inner.cfg.server_id, &inner.cfg.domain),
                    dest,
                    inner.ep.next_cseq(),
                    body,
                )
                .await
                {
                    tracing::warn!(error = %e, "gb28181 client: keepalive send failed");
                }
            }
        });
    }

    pub fn shutdown(&self) {
        self.inner.ep.shutdown();
    }
}

impl Drop for GbClient {
    /// Fire the cancel token so the cancel-aware pump tasks exit and release
    /// their `Arc<ClientInner>`, letting the `SipEndpoint` (and its UDP socket)
    /// drop. `cancel()` is idempotent, so an earlier `shutdown()` stays safe.
    fn drop(&mut self) {
        self.inner.ep.shutdown();
    }
}

async fn main_loop(inner: Arc<ClientInner>, mut incoming: TransactionReceiver) {
    loop {
        // Cancel-aware: the sender lives inside the Arc<ClientInner> this task
        // holds, so a plain recv() loop would pin the endpoint's UDP socket
        // open forever. Exit on cancel so the Arc can be reclaimed.
        let mut tx = tokio::select! {
            _ = inner.ep.cancel.cancelled() => return,
            msg = incoming.recv() => match msg {
                Some(tx) => tx,
                None => return,
            },
        };
        let has_to_tag = tx
            .original
            .to_header()
            .ok()
            .and_then(|t| t.tag().ok().flatten())
            .is_some();
        if has_to_tag {
            match inner.ep.dialog_layer.match_dialog(&tx) {
                Some(mut d) => {
                    tokio::spawn(async move {
                        d.handle(&mut tx).await.ok();
                    });
                }
                None => {
                    tx.reply(rsip::StatusCode::CallTransactionDoesNotExist)
                        .await
                        .ok();
                }
            }
            continue;
        }
        let inner = inner.clone();
        tokio::spawn(async move {
            let result = match tx.original.method {
                rsip::Method::Invite => handle_invite(&inner, &mut tx).await,
                rsip::Method::Message => handle_client_message(&inner, &mut tx).await,
                rsip::Method::Ack => Ok(()),
                _ => tx.reply(rsip::StatusCode::OK).await.map_err(sip_err),
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, "gb28181 client: request handling failed");
            }
        });
    }
}

async fn handle_invite(inner: &Arc<ClientInner>, tx: &mut Transaction) -> Result<()> {
    let contact: rsip::Uri = format!("sip:{}@{}", inner.cfg.device_id, inner.ep.local_addr)
        .try_into()
        .map_err(sip_err)?;
    match inner.ep.dialog_layer.get_or_create_server_invite(
        tx,
        inner.ep.state_sender.clone(),
        None,
        Some(contact),
    ) {
        Ok(mut dialog) => {
            // handle() drives the INVITE transaction (100/ringing/final) until
            // accept()/reject() is called on the dialog from the event consumer.
            dialog.handle(tx).await.map_err(sip_err)?;
            Ok(())
        }
        Err(_) => tx
            .reply(rsip::StatusCode::CallTransactionDoesNotExist)
            .await
            .map_err(sip_err),
    }
}

async fn handle_client_message(inner: &Arc<ClientInner>, tx: &mut Transaction) -> Result<()> {
    let body = tx.original.body.clone();
    match manscdp::peek_cmd_type(&body) {
        Ok(CmdType::Catalog) => {
            // A Catalog QUERY from the platform: ack it, then send our
            // channel list back as a separate MESSAGE.
            let (sn, _target) = manscdp::decode_sn_device(&body)?;
            tx.reply(rsip::StatusCode::OK).await.map_err(sip_err)?;
            let resp =
                manscdp::encode_catalog_response(sn, &inner.cfg.device_id, &inner.cfg.channels);
            send_platform_message(inner, resp).await
        }
        Ok(CmdType::DeviceInfo) => {
            // DeviceInfo QUERY: ack, then answer with our device metadata.
            let (sn, target) = manscdp::decode_sn_device(&body)?;
            tx.reply(rsip::StatusCode::OK).await.map_err(sip_err)?;
            let device_id = if target.is_empty() {
                &inner.cfg.device_id
            } else {
                &target
            };
            let resp = manscdp::encode_deviceinfo_response(
                sn,
                device_id,
                &inner.cfg.device_name,
                &inner.cfg.manufacturer,
                &inner.cfg.model,
                &inner.cfg.firmware,
            );
            send_platform_message(inner, resp).await
        }
        Ok(CmdType::RecordInfo) => {
            // RecordInfo QUERY (设备录像文件查询): ack, then list our recordings.
            let q = manscdp::decode_recordinfo_query(&body)?;
            tx.reply(rsip::StatusCode::OK).await.map_err(sip_err)?;
            let device_id = if q.device_id.is_empty() {
                &inner.cfg.device_id
            } else {
                &q.device_id
            };
            let resp = manscdp::encode_recordinfo_response(
                q.sn,
                device_id,
                &inner.cfg.device_name,
                &inner.cfg.records,
            );
            send_platform_message(inner, resp).await
        }
        Ok(CmdType::DeviceControl) => {
            // Device role: ack, then surface the raw PTZCmd for the consumer.
            tx.reply(rsip::StatusCode::OK).await.map_err(sip_err)?;
            match manscdp::decode_device_control(&body) {
                Ok(dc) => {
                    inner
                        .events
                        .send(GbEvent::DeviceControlReceived {
                            device_id: dc.device_id,
                            ptz_cmd: dc.ptz_cmd,
                        })
                        .ok();
                }
                Err(e) => tracing::debug!(error = %e, "gb28181 client: undecodable DeviceControl"),
            }
            Ok(())
        }
        _ => tx.reply(rsip::StatusCode::OK).await.map_err(sip_err),
    }
}

/// Send a MANSCDP MESSAGE (a query response) up to the platform, out of dialog.
async fn send_platform_message(inner: &Arc<ClientInner>, body: String) -> Result<()> {
    let dest = SipAddr::from(inner.cfg.server_addr);
    send_out_of_dialog_message(
        &inner.ep.inner(),
        (&inner.cfg.device_id, &inner.cfg.domain),
        (&inner.cfg.server_id, &inner.cfg.domain),
        dest,
        inner.ep.next_cseq(),
        body,
    )
    .await
    .map(|_| ())
}

async fn state_loop(
    inner: Arc<ClientInner>,
    mut state_rx: rsipstack::dialog::dialog::DialogStateReceiver,
) {
    loop {
        // Cancel-aware for the same reason as main_loop (see there).
        let state = tokio::select! {
            _ = inner.ep.cancel.cancelled() => return,
            msg = state_rx.recv() => match msg {
                Some(state) => state,
                None => return,
            },
        };
        match state {
            DialogState::Calling(id) => {
                let Some(Dialog::ServerInvite(dialog)) = inner.ep.dialog_layer.get_dialog(&id)
                else {
                    continue;
                };
                let req = dialog.initial_request();
                let offer_text = String::from_utf8_lossy(&req.body).to_string();
                match sdp::parse_offer(&offer_text) {
                    Ok(remote) => {
                        // Hand the INVITE to the event consumer; it answers or
                        // rejects via the negotiation.
                        emit_invite(inner.clone(), dialog, remote);
                    }
                    Err(_) => {
                        dialog.reject(None, Some("bad sdp".into())).ok();
                    }
                }
            }
            DialogState::Terminated(id, _reason) => {
                // Only surface SessionClosed for INVITEs we actually accepted;
                // rejected dialogs (e.g. bad-SDP 4xx) terminate silently.
                let key = id.to_string();
                if inner.accepted.lock().unwrap().remove(&key) {
                    inner
                        .events
                        .send(GbEvent::SessionClosed { dialog_id: key })
                        .ok();
                }
                inner.ep.dialog_layer.remove_dialog(&id);
            }
            _ => {}
        }
    }
}

fn emit_invite(inner: Arc<ClientInner>, dialog: ServerInviteDialog, remote: sdp::OfferSdp) {
    let negotiation = InviteNegotiation {
        dialog,
        device_id: inner.cfg.device_id.clone(),
        accepted: inner.accepted.clone(),
        remote,
    };
    inner.events.send(GbEvent::InviteReceived(negotiation)).ok();
}

/// An incoming INVITE pending an answer. Media is the consumer's job: open
/// your sender first, then `answer(...)` with where you'll send FROM.
pub struct InviteNegotiation {
    dialog: ServerInviteDialog,
    device_id: String,
    /// Accepted-dialog registry shared with the client's state loop: the
    /// dialog id is recorded here on `answer` so its termination surfaces a
    /// `SessionClosed` event (rejected dialogs stay out of it).
    accepted: Arc<Mutex<HashSet<String>>>,
    /// The offerer's parsed SDP: where IT wants to receive, its SSRC, transport.
    pub remote: sdp::OfferSdp,
}

impl std::fmt::Debug for InviteNegotiation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InviteNegotiation")
            .field("device_id", &self.device_id)
            .field("remote", &self.remote)
            .finish_non_exhaustive()
    }
}

impl InviteNegotiation {
    pub fn dialog_id(&self) -> String {
        self.dialog.id().to_string()
    }

    /// Accept: send 200 with an answer SDP naming `media_addr` as our RTP
    /// source. Echoes the offer's SSRC (`y=`).
    pub fn answer(self, media_addr: SocketAddr) -> Result<ClientMediaHandle> {
        let ssrc = self.remote.ssrc.clone().unwrap_or_default();
        let body = sdp::build_answer(
            &self.device_id,
            &media_addr.ip().to_string(),
            media_addr.port(),
            &ssrc,
            self.remote.transport,
            &self.remote.session,
            self.remote.start,
            self.remote.stop,
        );
        self.dialog
            .accept(
                Some(vec![rsip::Header::ContentType("application/sdp".into())]),
                Some(body.into_bytes()),
            )
            .map_err(sip_err)?;
        // Track only after a successful accept, so the state loop emits
        // SessionClosed for this dialog when it later terminates.
        self.accepted
            .lock()
            .unwrap()
            .insert(self.dialog.id().to_string());
        Ok(ClientMediaHandle {
            dialog: self.dialog,
        })
    }

    pub fn reject(self) -> Result<()> {
        self.dialog.reject(None, None).map_err(sip_err)
    }
}

/// The accepted media dialog, client side. BYE it when the stream stops.
pub struct ClientMediaHandle {
    dialog: ServerInviteDialog,
}

impl ClientMediaHandle {
    pub fn dialog_id(&self) -> String {
        self.dialog.id().to_string()
    }

    pub async fn bye(&self) -> Result<()> {
        self.dialog.bye().await.map_err(sip_err)
    }
}

#[cfg(test)]
#[path = "client_test.rs"]
mod client_test;
