//! Reverse proxy for ZLMediaKit's HTTP service, mounted under `/media`.
//!
//! Every request whose path starts with `/media` is forwarded to ZLM's HTTP
//! server (see `zlm::server` → `http_server_start(8553, false)`), with the
//! `/media` mount prefix stripped: `/media/rtp/x.live.flv` reaches ZLM's
//! `/rtp/x.live.flv`. Both plain HTTP — including long-lived HTTP-FLV and HLS,
//! whose responses are streamed, not buffered — and WebSocket (ZLM's WS-FLV)
//! are supported.

use std::sync::LazyLock;

use axum::{
    Router,
    body::Body,
    extract::{
        FromRequestParts, Request,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, HeaderName, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use futures::{SinkExt, StreamExt};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use tokio_tungstenite::tungstenite;

/// ZLM HTTP/WS endpoint. Loopback is fine — ZLM's server binds all interfaces.
const ZLM_HTTP_HOST: &str = "127.0.0.1";
const ZLM_HTTP_PORT: u16 = 8553;

/// Low-level client for the upstream leg. It forwards the incoming `Request`
/// after only rewriting its URI — request and response bodies stream through
/// untouched (no buffering), so uploads and live FLV/HLS are both unbounded.
static CLIENT: LazyLock<Client<HttpConnector, Body>> =
    LazyLock::new(|| Client::builder(TokioExecutor::new()).build_http());

/// Router that forwards `/media` and `/media/*` to ZLM. Merge into the app.
pub(crate) fn media_proxy_router() -> Router {
    Router::new()
        .route("/media", any(proxy))
        .route("/media/{*path}", any(proxy))
}

async fn proxy(req: Request) -> Response {
    let (mut parts, body) = req.into_parts();

    // ZLM target path = request path with the `/media` mount prefix removed.
    let (zlm_path, query) = {
        let path = parts.uri.path();
        let stripped = path.strip_prefix("/media").unwrap_or(path);
        let zlm_path = if stripped.is_empty() { "/" } else { stripped }.to_owned();
        (zlm_path, parts.uri.query().map(str::to_owned))
    };

    // A WebSocket upgrade request extracts cleanly; anything else is plain HTTP.
    match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(ws) => proxy_ws(ws, &zlm_path, query.as_deref()),
        Err(_) => {
            let req = Request::from_parts(parts, body);
            proxy_http(req, &zlm_path, query.as_deref()).await
        }
    }
}

/// Remove connection-specific (hop-by-hop) headers that must not cross a proxy.
fn strip_hop_by_hop(headers: &mut HeaderMap) {
    for name in [
        header::CONNECTION,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::TE,
        header::TRAILER,
    ] {
        headers.remove(name);
    }
    headers.remove(HeaderName::from_static("keep-alive"));
}

async fn proxy_http(mut req: Request, zlm_path: &str, query: Option<&str>) -> Response {
    // Point the request at ZLM; its body streams through untouched.
    let mut target = format!("http://{ZLM_HTTP_HOST}:{ZLM_HTTP_PORT}{zlm_path}");
    if let Some(q) = query {
        target.push('?');
        target.push_str(q);
    }
    match target.parse() {
        Ok(uri) => *req.uri_mut() = uri,
        Err(e) => {
            log::warn!("media proxy: bad target uri {target}: {e}");
            return StatusCode::BAD_GATEWAY.into_response();
        }
    }
    // The client derives Host from the new authority; drop the incoming one.
    req.headers_mut().remove(header::HOST);
    strip_hop_by_hop(req.headers_mut());

    match CLIENT.request(req).await {
        Ok(resp) => {
            let (mut parts, body) = resp.into_parts();
            strip_hop_by_hop(&mut parts.headers);
            Response::from_parts(parts, Body::new(body))
        }
        Err(e) => {
            log::warn!("media proxy: upstream error for {target}: {e}");
            (StatusCode::BAD_GATEWAY, "zlm upstream error").into_response()
        }
    }
}

fn proxy_ws(ws: WebSocketUpgrade, zlm_path: &str, query: Option<&str>) -> Response {
    let mut url = format!("ws://{ZLM_HTTP_HOST}:{ZLM_HTTP_PORT}{zlm_path}");
    if let Some(q) = query {
        url.push('?');
        url.push_str(q);
    }
    ws.on_upgrade(move |client| async move {
        if let Err(e) = relay_ws(client, &url).await {
            log::debug!("media ws proxy for {url} ended: {e}");
        }
    })
}

/// Bridge the client WebSocket to an upstream WS connection to ZLM, relaying
/// frames both ways until either side closes.
async fn relay_ws(client: WebSocket, url: &str) -> anyhow::Result<()> {
    let (upstream, _resp) = tokio_tungstenite::connect_async(url).await?;
    let (mut up_tx, mut up_rx) = upstream.split();
    let (mut cl_tx, mut cl_rx) = client.split();

    let client_to_upstream = async {
        while let Some(msg) = cl_rx.next().await {
            up_tx.send(axum_to_ts(msg?)).await?;
        }
        anyhow::Ok(())
    };
    let upstream_to_client = async {
        while let Some(msg) = up_rx.next().await {
            if let Some(m) = ts_to_axum(msg?) {
                cl_tx.send(m).await?;
            }
        }
        anyhow::Ok(())
    };

    tokio::select! {
        r = client_to_upstream => r,
        r = upstream_to_client => r,
    }
}

fn axum_to_ts(m: Message) -> tungstenite::Message {
    use tungstenite::Message as T;
    match m {
        Message::Text(t) => T::Text(t.as_str().to_owned().into()),
        Message::Binary(b) => T::Binary(b),
        Message::Ping(b) => T::Ping(b),
        Message::Pong(b) => T::Pong(b),
        Message::Close(c) => T::Close(c.map(|f| tungstenite::protocol::CloseFrame {
            code: f.code.into(),
            reason: f.reason.as_str().to_owned().into(),
        })),
    }
}

fn ts_to_axum(m: tungstenite::Message) -> Option<Message> {
    use tungstenite::Message as T;
    Some(match m {
        T::Text(t) => Message::Text(t.as_str().to_owned().into()),
        T::Binary(b) => Message::Binary(b),
        T::Ping(b) => Message::Ping(b),
        T::Pong(b) => Message::Pong(b),
        T::Close(c) => Message::Close(c.map(|f| axum::extract::ws::CloseFrame {
            code: f.code.into(),
            reason: f.reason.as_str().to_owned().into(),
        })),
        // Raw frames are internal to tungstenite; safe to drop (per maintainers).
        T::Frame(_) => return None,
    })
}
