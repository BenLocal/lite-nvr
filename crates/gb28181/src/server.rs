//! GbServer — the UAS / 上级平台 facade (spec §5.2).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use rsipstack::dialog::client_dialog::ClientInviteDialog;
use rsipstack::dialog::dialog::DialogState;
use rsipstack::dialog::invitation::InviteOption;
use rsipstack::sip as rsip;
use rsipstack::sip::prelude::{HeadersExt, ToTypedHeader};
use rsipstack::transaction::TransactionReceiver;
use rsipstack::transaction::transaction::Transaction;
use rsipstack::transport::SipAddr;
use tokio::sync::mpsc;

use crate::auth::{AuthConfig, AuthDecision};
use crate::endpoint::{SipEndpoint, send_out_of_dialog_message, sip_err};
use crate::error::{GbError, Result};
use crate::event::GbEvent;
use crate::gbcode::{SsrcGenerator, SsrcKind};
use crate::manscdp::{self, Catalog, CatalogAccumulator, CmdType};
use crate::registrar::{Registrar, RegistrarChange, SweepOutcome};
use crate::sdp;
use crate::types::{MediaSpec, RegisteredDevice, Transport};

pub struct GbServerConfig {
    /// Our 20-digit platform GB code (e.g. "34020000002000000001").
    pub sip_id: String,
    /// SIP domain / digest realm (e.g. "3402000000").
    pub domain: String,
    /// UDP listen address (port 0 = ephemeral, for tests).
    pub listen: SocketAddr,
    pub auth: AuthConfig,
    /// Seconds without keepalive before a device is marked Offline.
    pub keepalive_grace: i64,
    pub sweep_interval: Duration,
    /// Total window for aggregating a multi-chunk Catalog reply.
    pub query_timeout: Duration,
    pub user_agent: String,
}

impl GbServerConfig {
    pub fn new(sip_id: impl Into<String>, domain: impl Into<String>, listen: SocketAddr) -> Self {
        Self {
            sip_id: sip_id.into(),
            domain: domain.into(),
            listen,
            auth: AuthConfig::Open,
            keepalive_grace: 180,
            sweep_interval: Duration::from_secs(10),
            query_timeout: Duration::from_secs(8),
            user_agent: "lite-nvr-gb28181".into(),
        }
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn md5_hex(input: &[u8]) -> String {
    let d = Md5::digest(input);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

struct ServerInner {
    cfg: GbServerConfig,
    ep: SipEndpoint,
    registrar: Mutex<Registrar>,
    /// device_id -> where its last REGISTER came from (reply/INVITE target).
    dests: Mutex<HashMap<String, SipAddr>>,
    /// Issued digest nonces -> issue time (single use, 300s validity).
    nonces: Mutex<HashMap<String, i64>>,
    /// Catalog SN -> chunk router for an in-flight query.
    pending_catalog: Mutex<HashMap<u64, mpsc::UnboundedSender<manscdp::CatalogResponse>>>,
    /// Confirmed media dialog ids we own (for SessionClosed events).
    sessions: Mutex<HashMap<String, ()>>,
    events: mpsc::UnboundedSender<GbEvent>,
    /// Query SN counter (Catalog/DeviceInfo).
    sn: AtomicU64,
    nonce_seq: AtomicU64,
    ssrc: SsrcGenerator,
    /// Dropped-but-not-stopped sessions get their dialog sent here for BYE
    /// (see `MediaSession::drop`).
    janitor: mpsc::UnboundedSender<ClientInviteDialog>,
}

pub struct GbServer {
    inner: Arc<ServerInner>,
}

/// An owned outbound media dialog (RAII, spec §3 row 5): `stop()` sends BYE;
/// dropping without `stop()` enqueues a best-effort BYE via the janitor.
pub struct MediaSession {
    dialog: ClientInviteDialog,
    pub spec: MediaSpec,
    stopped: bool,
    janitor: mpsc::UnboundedSender<ClientInviteDialog>,
}

impl MediaSession {
    pub fn dialog_id(&self) -> String {
        self.dialog.id().to_string()
    }

    pub async fn stop(mut self) -> Result<()> {
        self.stopped = true;
        self.dialog.bye().await.map_err(sip_err)
    }
}

impl Drop for MediaSession {
    fn drop(&mut self) {
        if !self.stopped {
            self.janitor.send(self.dialog.clone()).ok();
        }
    }
}

impl GbServer {
    /// Bind + start all pump tasks. The receiver is the single event stream.
    pub async fn bind(cfg: GbServerConfig) -> Result<(GbServer, mpsc::UnboundedReceiver<GbEvent>)> {
        let (ep, streams) = SipEndpoint::bind_udp(cfg.listen, &cfg.user_agent).await?;
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let (janitor_tx, janitor_rx) = mpsc::unbounded_channel();
        let keepalive_grace = cfg.keepalive_grace;
        let ssrc = SsrcGenerator::new(&cfg.sip_id);
        let inner = Arc::new(ServerInner {
            cfg,
            ep,
            registrar: Mutex::new(Registrar::new(keepalive_grace)),
            dests: Mutex::new(HashMap::new()),
            nonces: Mutex::new(HashMap::new()),
            pending_catalog: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            events: events_tx,
            sn: AtomicU64::new(1),
            nonce_seq: AtomicU64::new(1),
            ssrc,
            janitor: janitor_tx,
        });
        tokio::spawn(main_loop(inner.clone(), streams.incoming));
        tokio::spawn(state_loop(inner.clone(), streams.state_receiver));
        tokio::spawn(sweep_loop(inner.clone()));
        tokio::spawn(janitor_loop(janitor_rx));
        Ok((GbServer { inner }, events_rx))
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.inner.ep.local_addr
    }

    pub fn devices(&self) -> Vec<RegisteredDevice> {
        self.inner.registrar.lock().unwrap().list()
    }

    /// Allocate the next GB-format SSRC for a caller building a MediaSpec.
    pub fn next_ssrc(&self, kind: SsrcKind) -> (u32, String) {
        self.inner.ssrc.next(kind)
    }

    /// MANSCDP Catalog query: send, then aggregate response chunks by SN
    /// until SumNum is reached or `query_timeout` elapses. Partial results
    /// come back with `incomplete = true` (spec §3 row 9).
    pub async fn catalog_query(&self, device_id: &str) -> Result<Catalog> {
        let inner = &self.inner;
        let dest = inner
            .dests
            .lock()
            .unwrap()
            .get(device_id)
            .cloned()
            .ok_or_else(|| GbError::DeviceOffline(device_id.to_string()))?;
        let sn = inner.sn.fetch_add(1, Ordering::Relaxed);
        let (chunk_tx, mut chunk_rx) = mpsc::unbounded_channel();
        inner.pending_catalog.lock().unwrap().insert(sn, chunk_tx);

        let body = manscdp::encode_catalog_query(sn, device_id);
        // Hoist the endpoint ref: `&inner.ep.inner()` inline would create a
        // temporary that the returned future outlives (rsipstack 0.5.3 E0515).
        let ep = inner.ep.inner();
        let send = || {
            send_out_of_dialog_message(
                &ep,
                (&inner.cfg.sip_id, &inner.cfg.domain),
                (device_id, &inner.cfg.domain),
                dest.clone(),
                inner.ep.next_cseq(),
                body.clone(),
            )
        };
        // Auto-retry once on transport/transaction failure (spec §3 row 9).
        let sent = match send().await {
            Ok(resp) => Ok(resp),
            Err(_) => send().await,
        };
        if let Err(e) = sent {
            inner.pending_catalog.lock().unwrap().remove(&sn);
            return Err(e);
        }

        let mut acc = CatalogAccumulator::new(sn);
        let deadline = tokio::time::Instant::now() + inner.cfg.query_timeout;
        // A non-`Ok(Some)` (channel closed or `query_timeout` elapsed) ends the loop.
        while let Ok(Some(chunk)) = tokio::time::timeout_at(deadline, chunk_rx.recv()).await {
            acc.push(chunk);
            if acc.is_complete() {
                break;
            }
        }
        inner.pending_catalog.lock().unwrap().remove(&sn);
        Ok(acc.finish())
    }

    /// INVITE `channel_id` on `device_id` to start pushing media to
    /// `spec.media_addr` (the caller's already-open receiver). On 200 OK the
    /// answer's media address lands in `spec.negotiated_remote` (needed for
    /// TcpActive connect).
    pub async fn invite_play(
        &self,
        device_id: &str,
        channel_id: &str,
        mut spec: MediaSpec,
    ) -> Result<MediaSession> {
        let inner = &self.inner;
        let dest = inner
            .dests
            .lock()
            .unwrap()
            .get(device_id)
            .cloned()
            .ok_or_else(|| GbError::DeviceOffline(device_id.to_string()))?;

        let offer = sdp::build_play_offer(
            &inner.cfg.sip_id,
            &spec.media_addr.ip().to_string(),
            spec.media_addr.port(),
            &spec.ssrc_str,
            spec.transport,
            &spec.stream_type,
        );
        let caller: rsip::Uri = format!("sip:{}@{}", inner.cfg.sip_id, inner.cfg.domain)
            .try_into()
            .map_err(sip_err)?;
        let callee: rsip::Uri = format!("sip:{}@{}", channel_id, inner.cfg.domain)
            .try_into()
            .map_err(sip_err)?;
        let contact: rsip::Uri = format!("sip:{}@{}", inner.cfg.sip_id, inner.ep.local_addr)
            .try_into()
            .map_err(sip_err)?;
        // GB/T 28181 Subject: 发送者媒体流序列号,接收者标识 — channel:ssrc,platform:0
        let subject = format!("{}:{},{}:0", channel_id, spec.ssrc_str, inner.cfg.sip_id);
        let opt = InviteOption {
            caller,
            callee,
            contact,
            content_type: Some("application/sdp".to_string()),
            offer: Some(offer.into_bytes()),
            destination: Some(dest),
            headers: Some(vec![rsip::Header::Subject(subject.into())]),
            credential: None,
            ..Default::default()
        };
        let (dialog, resp) = inner
            .ep
            .dialog_layer
            .do_invite(opt, inner.ep.state_sender.clone())
            .await
            .map_err(sip_err)?;
        let resp = resp.ok_or_else(|| GbError::Negotiation("no INVITE response".into()))?;
        if resp.status_code != rsip::StatusCode::OK {
            return Err(GbError::Negotiation(format!(
                "INVITE rejected: {}",
                resp.status_code
            )));
        }
        let answer_text = String::from_utf8_lossy(resp.body()).to_string();
        let answer = sdp::parse_answer(&answer_text)?;
        spec.negotiated_remote = Some(answer.media_addr);

        inner
            .sessions
            .lock()
            .unwrap()
            .insert(dialog.id().to_string(), ());
        Ok(MediaSession {
            dialog,
            spec,
            stopped: false,
            janitor: inner.janitor.clone(),
        })
    }

    pub fn shutdown(&self) {
        self.inner.ep.shutdown();
    }
}

impl Drop for GbServer {
    /// The pump tasks each hold an `Arc<ServerInner>`, so dropping this handle
    /// alone can't reclaim the endpoint. Fire the cancel token so the (now
    /// cancel-aware) pump tasks exit and release their `Arc`, letting the
    /// `SipEndpoint` — and its UDP socket — drop. `cancel()` is idempotent, so
    /// an earlier explicit `shutdown()` stays safe.
    fn drop(&mut self) {
        self.inner.ep.shutdown();
    }
}

async fn main_loop(inner: Arc<ServerInner>, mut incoming: TransactionReceiver) {
    loop {
        // Cancel-aware: without this the task would block on recv() forever
        // (its sender lives inside the Arc<ServerInner> this task holds), so
        // the Arc — and the UDP socket it owns — could never be reclaimed.
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
                rsip::Method::Register => handle_register(&inner, &mut tx).await,
                rsip::Method::Message => handle_message(&inner, &mut tx).await,
                rsip::Method::Ack => Ok(()), // stray ACK, nothing to do
                _ => tx
                    .reply(rsip::StatusCode::MethodNotAllowed)
                    .await
                    .map_err(sip_err),
            };
            if let Err(e) = result {
                tracing::warn!(error = %e, "gb28181 server: request handling failed");
            }
        });
    }
}

fn request_user(req: &rsip::Request) -> Option<String> {
    req.from_header()
        .ok()?
        .typed()
        .ok()?
        .uri
        .auth
        .map(|a| a.user)
}

async fn handle_register(inner: &Arc<ServerInner>, tx: &mut Transaction) -> Result<()> {
    let Some(device_id) = request_user(&tx.original) else {
        tx.reply(rsip::StatusCode::BadRequest)
            .await
            .map_err(sip_err)?;
        return Ok(());
    };
    match inner.cfg.auth.password_for(&device_id) {
        AuthDecision::Allow => {}
        AuthDecision::Reject => {
            tx.reply(rsip::StatusCode::Forbidden)
                .await
                .map_err(sip_err)?;
            return Ok(());
        }
        AuthDecision::Require(password) => {
            if !check_authorization(inner, tx, &password) {
                return challenge(inner, tx).await;
            }
        }
    }

    let expires: i64 = tx
        .original
        .expires_header()
        .and_then(|e| e.value().trim().parse().ok())
        .unwrap_or(3600);
    let contact = tx
        .original
        .contact_header()
        .map(|c| c.to_string())
        .unwrap_or_default();
    let change = {
        let mut r = inner.registrar.lock().unwrap();
        r.register(&device_id, &contact, Transport::Udp, expires, now_unix())
    };
    if expires > 0 {
        if let Some(dest) = inner
            .ep
            .inner()
            .get_destination_from_request(&tx.original)
            .await
        {
            inner.dests.lock().unwrap().insert(device_id.clone(), dest);
        }
    } else {
        inner.dests.lock().unwrap().remove(&device_id);
    }

    let date = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let headers = vec![
        rsip::Header::Expires((expires.max(0) as u32).into()),
        rsip::Header::Date(date.into()),
    ];
    tx.reply_with(rsip::StatusCode::OK, headers, None)
        .await
        .map_err(sip_err)?;

    match change {
        RegistrarChange::Registered => {
            inner.events.send(GbEvent::Registered { device_id }).ok();
        }
        RegistrarChange::Unregistered => {
            inner.events.send(GbEvent::Unregistered { device_id }).ok();
        }
        RegistrarChange::Refreshed | RegistrarChange::NoChange => {}
    }
    Ok(())
}

/// True iff the request carries a digest Authorization with a nonce we
/// issued (single-use) and a response matching `password`.
fn check_authorization(inner: &Arc<ServerInner>, tx: &Transaction, password: &str) -> bool {
    // rsipstack 0.5.3 does not implement `ToTypedHeader` for `Authorization`
    // (unlike From/To/Via), so parse the untyped value into `typed::Authorization`.
    let Some(auth) = tx.original.headers.iter().find_map(|h| match h {
        rsip::Header::Authorization(a) => rsip::typed::Authorization::parse(a.value()).ok(),
        _ => None,
    }) else {
        return false;
    };
    let nonce_ok = {
        let mut nonces = inner.nonces.lock().unwrap();
        let now = now_unix();
        nonces.retain(|_, issued| now - *issued < 300);
        nonces.remove(&auth.nonce).is_some()
    };
    if !nonce_ok {
        return false;
    }
    crate::auth::verify(
        &auth.username,
        &auth.realm,
        password,
        "REGISTER",
        &auth.uri.to_string(),
        &auth.nonce,
        &auth.response,
    )
}

async fn challenge(inner: &Arc<ServerInner>, tx: &mut Transaction) -> Result<()> {
    let seq = inner.nonce_seq.fetch_add(1, Ordering::Relaxed);
    let seed = format!(
        "{}:{}:{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        seq,
        inner.cfg.sip_id
    );
    let nonce = md5_hex(seed.as_bytes());
    inner
        .nonces
        .lock()
        .unwrap()
        .insert(nonce.clone(), now_unix());
    let value = format!(
        r#"Digest realm="{}", nonce="{}", algorithm=MD5"#,
        inner.cfg.domain, nonce
    );
    tx.reply_with(
        rsip::StatusCode::Unauthorized,
        vec![rsip::Header::WwwAuthenticate(value.into())],
        None,
    )
    .await
    .map_err(sip_err)
}

async fn handle_message(inner: &Arc<ServerInner>, tx: &mut Transaction) -> Result<()> {
    let body = tx.original.body.clone();
    match manscdp::peek_cmd_type(&body) {
        Ok(CmdType::Keepalive) => {
            let ka = manscdp::decode_keepalive(&body)?;
            let known = inner
                .registrar
                .lock()
                .unwrap()
                .keepalive(&ka.device_id, now_unix());
            if known {
                tx.reply(rsip::StatusCode::OK).await.map_err(sip_err)?;
                inner
                    .events
                    .send(GbEvent::KeepaliveReceived {
                        device_id: ka.device_id,
                    })
                    .ok();
            } else {
                tx.reply(rsip::StatusCode::NotFound)
                    .await
                    .map_err(sip_err)?;
            }
        }
        Ok(CmdType::Catalog) => {
            // A Catalog RESPONSE chunk from a device we queried.
            let resp = manscdp::decode_catalog_response(&body)?;
            tx.reply(rsip::StatusCode::OK).await.map_err(sip_err)?;
            let sender = inner.pending_catalog.lock().unwrap().get(&resp.sn).cloned();
            if let Some(sender) = sender {
                sender.send(resp).ok();
            }
        }
        _ => {
            // Lenient policy (spec §3 row 9): acknowledge what we don't handle.
            tx.reply(rsip::StatusCode::OK).await.map_err(sip_err)?;
        }
    }
    Ok(())
}

async fn state_loop(
    inner: Arc<ServerInner>,
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
        if let DialogState::Terminated(id, _reason) = state {
            let key = id.to_string();
            let owned = inner.sessions.lock().unwrap().remove(&key).is_some();
            if owned {
                inner
                    .events
                    .send(GbEvent::SessionClosed { dialog_id: key })
                    .ok();
            }
            inner.ep.dialog_layer.remove_dialog(&id);
        }
    }
}

async fn sweep_loop(inner: Arc<ServerInner>) {
    let mut tick = tokio::time::interval(inner.cfg.sweep_interval);
    loop {
        tokio::select! {
            _ = inner.ep.cancel.cancelled() => return,
            _ = tick.tick() => {}
        }
        let now = now_unix();
        // Prune expired digest nonces here too, so a REGISTER flood that never
        // completes auth can't grow the nonce map without bound.
        inner
            .nonces
            .lock()
            .unwrap()
            .retain(|_, issued| now - *issued < 300);
        // sweep() decides Offline vs Dropped atomically under its own lock, so
        // no racy follow-up get() is needed to classify each change.
        let changed = {
            let mut r = inner.registrar.lock().unwrap();
            r.sweep(now)
        };
        for (device_id, outcome) in changed {
            let event = match outcome {
                SweepOutcome::WentOffline => GbEvent::Offline { device_id },
                SweepOutcome::Dropped => {
                    inner.dests.lock().unwrap().remove(&device_id);
                    GbEvent::Unregistered { device_id }
                }
            };
            inner.events.send(event).ok();
        }
    }
}

async fn janitor_loop(mut rx: mpsc::UnboundedReceiver<ClientInviteDialog>) {
    while let Some(dialog) = rx.recv().await {
        dialog.bye().await.ok();
    }
}

// pub(crate): client_test.rs reuses wait_for/raw_register from here.
#[cfg(test)]
#[path = "server_test.rs"]
pub(crate) mod server_test;
