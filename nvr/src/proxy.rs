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
    http::{HeaderName, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite;

/// ZLM HTTP/WS endpoint. Loopback is fine — ZLM's server binds all interfaces.
const ZLM_HTTP_HOST: &str = "127.0.0.1";
const ZLM_HTTP_PORT: u16 = 8553;
/// Cap on a proxied *request* body we buffer before forwarding. Responses are
/// streamed (so live FLV/HLS are unbounded); this only limits uploads.
const MAX_REQUEST_BODY: usize = 64 * 1024 * 1024;

/// Shared HTTP client for the upstream leg. Redirects are disabled so the proxy
/// passes ZLM's responses through verbatim instead of chasing them itself.
static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("build zlm proxy http client")
});

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

/// Headers that are connection-specific and must not be forwarded either way.
fn is_hop_by_hop(name: &HeaderName) -> bool {
    *name == header::CONNECTION
        || *name == header::TRANSFER_ENCODING
        || *name == header::UPGRADE
        || *name == header::PROXY_AUTHENTICATE
        || *name == header::PROXY_AUTHORIZATION
        || *name == header::TE
        || *name == header::TRAILER
        || name.as_str().eq_ignore_ascii_case("keep-alive")
}

async fn proxy_http(req: Request, zlm_path: &str, query: Option<&str>) -> Response {
    let (parts, body) = req.into_parts();

    let mut url = format!("http://{ZLM_HTTP_HOST}:{ZLM_HTTP_PORT}{zlm_path}");
    if let Some(q) = query {
        url.push('?');
        url.push_str(q);
    }

    let body_bytes = match axum::body::to_bytes(body, MAX_REQUEST_BODY).await {
        Ok(b) => b,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };

    let mut rb = CLIENT.request(parts.method, &url);
    for (name, value) in parts.headers.iter() {
        if is_hop_by_hop(name) || *name == header::HOST {
            continue;
        }
        rb = rb.header(name, value);
    }
    if !body_bytes.is_empty() {
        rb = rb.body(body_bytes);
    }

    let upstream = match rb.send().await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("media proxy: upstream error for {url}: {e}");
            return (StatusCode::BAD_GATEWAY, "zlm upstream error").into_response();
        }
    };

    let mut builder = Response::builder().status(upstream.status());
    for (name, value) in upstream.headers().iter() {
        if is_hop_by_hop(name) {
            continue;
        }
        builder = builder.header(name, value);
    }
    builder
        .body(Body::from_stream(upstream.bytes_stream()))
        .unwrap_or_else(|e| {
            log::error!("media proxy: build response: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })
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
