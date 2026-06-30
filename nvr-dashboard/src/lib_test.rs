use crate::{AppAssets, app_router};
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt; // for `oneshot`

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

#[tokio::test]
async fn serves_index_with_no_cache() {
    let res = app_router(None).oneshot(get("/")).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers().get(header::CONTENT_TYPE).unwrap();
    assert!(
        ct.to_str().unwrap().starts_with("text/html"),
        "content-type was {ct:?}"
    );
    assert_eq!(
        res.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-cache"
    );
}

#[tokio::test]
async fn missing_asset_returns_404_not_index_html() {
    // The bug this guards against: a stale/missing chunk being served as the
    // index.html shell (text/html), which trips strict MIME checks.
    let res = app_router(None)
        .oneshot(get("/assets/does-not-exist-DEADBEEF.js"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unknown_spa_route_falls_back_to_index() {
    let res = app_router(None)
        .oneshot(get("/some/client/side/route"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers().get(header::CONTENT_TYPE).unwrap();
    assert!(
        ct.to_str().unwrap().starts_with("text/html"),
        "content-type was {ct:?}"
    );
}

#[tokio::test]
async fn real_js_asset_served_with_js_mime_and_immutable_cache() {
    let asset = AppAssets::iter()
        .find(|p| p.starts_with("assets/") && p.ends_with(".js"))
        .expect("expected at least one built JS asset in app/dist");
    let res = app_router(None)
        .oneshot(get(&format!("/{asset}")))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers().get(header::CONTENT_TYPE).unwrap();
    assert!(
        ct.to_str().unwrap().contains("javascript"),
        "content-type was {ct:?}"
    );
    assert_eq!(
        res.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn prefix_nested_router_strips_prefix() {
    let res = app_router(Some("/nvr"))
        .oneshot(get("/nvr/assets/does-not-exist-DEADBEEF.js"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn prefix_nested_root_serves_index() {
    // Visiting the dashboard root (`/nvr/`) must return the SPA shell, not 404.
    let res = app_router(Some("/nvr"))
        .oneshot(get("/nvr/"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers().get(header::CONTENT_TYPE).unwrap();
    assert!(
        ct.to_str().unwrap().starts_with("text/html"),
        "content-type was {ct:?}"
    );
}

#[tokio::test]
async fn prefix_nested_spa_route_serves_index() {
    // A client-side route under the prefix (e.g. after login → dashboard).
    let res = app_router(Some("/nvr"))
        .oneshot(get("/nvr/dashboard"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res.headers().get(header::CONTENT_TYPE).unwrap();
    assert!(
        ct.to_str().unwrap().starts_with("text/html"),
        "content-type was {ct:?}"
    );
}
