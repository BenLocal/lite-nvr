use axum::{
    Extension, Router,
    body::Body,
    http::{Request as HttpRequest, StatusCode},
    middleware,
    routing::get,
};
use chrono::{Duration, Utc};
use tower::ServiceExt;

use super::*;

/// Serializes the DB-writing tests: turso allows one WAL writer, and parallel
/// test bodies hitting the shared in-memory APP_DB otherwise fail with
/// intermittent "database is locked" (see the write-contention note in
/// `crate::db`).
static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Initialize the process-wide APP_DB once (all tests share one binary) with
/// an in-memory database carrying the `kvs` table sessions live in, and take
/// the serialization lock for the calling test.
async fn ensure_test_db() -> tokio::sync::MutexGuard<'static, ()> {
    static INIT: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    INIT.get_or_init(|| async {
        let db = crate::db::init_app_db(":memory:").await.unwrap();
        let conn = db.connect().unwrap();
        conn.execute_batch(
            r#"CREATE TABLE kvs (
                id INTEGER NOT NULL,
                module VARCHAR NOT NULL,
                key VARCHAR NOT NULL,
                sub_key VARCHAR NOT NULL,
                value TEXT NOT NULL,
                PRIMARY KEY(id AUTOINCREMENT)
            );"#,
        )
        .await
        .unwrap();
    })
    .await;
    DB_LOCK.lock().await
}

fn protected_app() -> Router {
    Router::new()
        .route("/user/login", get(async || "login"))
        .route(
            "/whoami",
            get(async |Extension(user): Extension<AuthUser>| user.username),
        )
        .layer(middleware::from_fn(require_auth))
}

async fn status_of(app: Router, req: HttpRequest<Body>) -> StatusCode {
    app.oneshot(req).await.unwrap().status()
}

#[test]
fn query_token_parses_pairs() {
    assert_eq!(query_token(Some("token=abc")), Some("abc".to_string()));
    assert_eq!(
        query_token(Some("a=1&token=abc&b=2")),
        Some("abc".to_string())
    );
    assert_eq!(query_token(Some("a=1&b=2")), None);
    assert_eq!(query_token(Some("token=")), None);
    assert_eq!(query_token(None), None);
}

#[tokio::test]
async fn middleware_rejects_missing_and_unknown_tokens() {
    let _db = ensure_test_db().await;

    let req = HttpRequest::get("/whoami").body(Body::empty()).unwrap();
    assert_eq!(
        status_of(protected_app(), req).await,
        StatusCode::UNAUTHORIZED
    );

    let req = HttpRequest::get("/whoami")
        .header("Authorization", "Bearer no-such-token")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        status_of(protected_app(), req).await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn middleware_exempts_login() {
    let _db = ensure_test_db().await;
    let req = HttpRequest::get("/user/login").body(Body::empty()).unwrap();
    assert_eq!(status_of(protected_app(), req).await, StatusCode::OK);
}

#[tokio::test]
async fn middleware_accepts_bearer_header_and_query_token() {
    let _db = ensure_test_db().await;
    let token = create_session("alice").await.unwrap();

    let req = HttpRequest::get("/whoami")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::empty())
        .unwrap();
    let res = protected_app().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], b"alice");

    let req = HttpRequest::get(format!("/whoami?token={}", token))
        .body(Body::empty())
        .unwrap();
    assert_eq!(status_of(protected_app(), req).await, StatusCode::OK);
}

#[tokio::test]
async fn validate_survives_cache_loss_and_gcs_expired_sessions() {
    let _db = ensure_test_db().await;
    let conn = crate::db::app_db_conn().unwrap();

    // A DB-only session (not in cache — as after a restart) still validates.
    let restored = nvr_db::session::Session {
        token: "restored-token".to_string(),
        username: "bob".to_string(),
        expires_at: Utc::now() + Duration::hours(1),
    };
    nvr_db::session::insert(&restored, &conn).await.unwrap();
    assert_eq!(validate("restored-token").await.as_deref(), Some("bob"));

    // An expired DB session is rejected and garbage-collected.
    let expired = nvr_db::session::Session {
        token: "expired-token".to_string(),
        username: "bob".to_string(),
        expires_at: Utc::now() - Duration::hours(1),
    };
    nvr_db::session::insert(&expired, &conn).await.unwrap();
    assert!(validate("expired-token").await.is_none());
    assert!(
        nvr_db::session::get_by_token("expired-token", &conn)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn revoke_user_spares_the_excepted_token() {
    let _db = ensure_test_db().await;
    let keep = create_session("carol").await.unwrap();
    let kick = create_session("carol").await.unwrap();

    revoke_user("carol", Some(&keep)).await.unwrap();

    assert_eq!(validate(&keep).await.as_deref(), Some("carol"));
    assert!(validate(&kick).await.is_none());
}

#[tokio::test]
async fn revoke_forgets_the_token() {
    let _db = ensure_test_db().await;
    let token = create_session("dave").await.unwrap();
    assert!(validate(&token).await.is_some());

    revoke(&token).await.unwrap();
    assert!(validate(&token).await.is_none());
}
