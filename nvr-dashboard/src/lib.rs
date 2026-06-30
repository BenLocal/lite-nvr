use axum::{
    Router,
    handler::HandlerWithoutStateExt,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::{EmbeddedFile, RustEmbed};

#[derive(RustEmbed)]
#[folder = "app/dist/"]
struct AppAssets;

pub fn app_router(prefix: Option<&str>) -> Router {
    // Serve via a `Service` (handler `.into_service()`), not a `fallback`
    // handler: when mounted under a prefix, axum dispatches `/{prefix}/`
    // (the bare dashboard root) to a nested *service* but not to a nested
    // fallback handler, so a handler-based fallback would 404 the SPA root.
    match prefix {
        Some(prefix) => Router::new().nest_service(prefix, serve_embedded.into_service()),
        None => Router::new().fallback_service(serve_embedded.into_service()),
    }
}

/// Serve the embedded SPA.
///
/// Real files (including hashed `assets/*`) are returned with their correct
/// MIME type. A miss on an asset-looking path returns a clean 404 instead of
/// the `index.html` shell — otherwise a stale chunk hash would be served as
/// HTML and rejected by the browser's strict MIME check for module scripts.
/// Any other miss falls back to `index.html` so client-side routing keeps
/// working on a hard refresh of a deep link.
async fn serve_embedded(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(file) = AppAssets::get(path) {
        return file_response(path, file);
    }

    if is_asset_request(path) {
        return StatusCode::NOT_FOUND.into_response();
    }

    match AppAssets::get("index.html") {
        Some(index) => file_response("index.html", index),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn file_response(path: &str, file: EmbeddedFile) -> Response {
    let content_type = HeaderValue::from_str(file.metadata.mimetype())
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    // index.html is never cached: it pins the hashed chunk URLs, so a stale copy
    // makes the browser load an old build (e.g. a previous LoginView with the
    // green `var(--p-primary-color)` background). `no-store` forces a fresh fetch
    // every load. Hashed assets are content-addressed, so they stay immutable.
    let cache_control = if path == "index.html" {
        HeaderValue::from_static("no-store, no-cache, must-revalidate")
    } else {
        HeaderValue::from_static("public, max-age=31536000, immutable")
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, cache_control),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

/// Paths that must resolve to a real file — never the SPA shell. Keeps a stale
/// or missing asset request honest (404) instead of masquerading as HTML.
fn is_asset_request(path: &str) -> bool {
    path.starts_with("assets/")
        || matches!(
            path.rsplit('.').next(),
            Some(
                "js" | "mjs"
                    | "css"
                    | "map"
                    | "json"
                    | "wasm"
                    | "woff"
                    | "woff2"
                    | "ttf"
                    | "otf"
                    | "eot"
                    | "ico"
                    | "png"
                    | "jpg"
                    | "jpeg"
                    | "gif"
                    | "svg"
                    | "webp"
                    | "avif"
            )
        )
}

#[cfg(test)]
#[path = "lib_test.rs"]
mod lib_test;
