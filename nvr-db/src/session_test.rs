use chrono::{Duration, Utc};
use turso::Connection;

use crate::db::{DatabaseConfig, NvrDatabase};
use crate::session::{self, Session};

async fn test_conn() -> Connection {
    let db = NvrDatabase::new(&DatabaseConfig::new(":memory:"))
        .await
        .unwrap();
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
    conn
}

fn session(token: &str, username: &str, ttl_hours: i64) -> Session {
    Session {
        token: token.to_string(),
        username: username.to_string(),
        expires_at: Utc::now() + Duration::hours(ttl_hours),
    }
}

#[tokio::test]
async fn insert_and_get_round_trip() {
    let conn = test_conn().await;
    session::insert(&session("tok-1", "alice", 1), &conn)
        .await
        .unwrap();

    let found = session::get_by_token("tok-1", &conn)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.token, "tok-1");
    assert_eq!(found.username, "alice");

    assert!(
        session::get_by_token("missing", &conn)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn delete_removes_only_that_token() {
    let conn = test_conn().await;
    session::insert(&session("tok-1", "alice", 1), &conn)
        .await
        .unwrap();
    session::insert(&session("tok-2", "alice", 1), &conn)
        .await
        .unwrap();

    session::delete("tok-1", &conn).await.unwrap();

    assert!(
        session::get_by_token("tok-1", &conn)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        session::get_by_token("tok-2", &conn)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn delete_by_username_spares_excepted_token() {
    let conn = test_conn().await;
    session::insert(&session("tok-1", "alice", 1), &conn)
        .await
        .unwrap();
    session::insert(&session("tok-2", "alice", 1), &conn)
        .await
        .unwrap();
    session::insert(&session("tok-3", "bob", 1), &conn)
        .await
        .unwrap();

    session::delete_by_username("alice", Some("tok-2"), &conn)
        .await
        .unwrap();

    assert!(
        session::get_by_token("tok-1", &conn)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        session::get_by_token("tok-2", &conn)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        session::get_by_token("tok-3", &conn)
            .await
            .unwrap()
            .is_some()
    );

    session::delete_by_username("alice", None, &conn)
        .await
        .unwrap();
    assert!(
        session::get_by_token("tok-2", &conn)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn delete_expired_keeps_live_sessions() {
    let conn = test_conn().await;
    session::insert(&session("live", "alice", 1), &conn)
        .await
        .unwrap();
    session::insert(&session("dead", "alice", -1), &conn)
        .await
        .unwrap();

    session::delete_expired(Utc::now(), &conn).await.unwrap();

    assert!(
        session::get_by_token("live", &conn)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        session::get_by_token("dead", &conn)
            .await
            .unwrap()
            .is_none()
    );
}
