use std::collections::HashMap;

use chrono::Utc;
use turso::Connection;

use crate::db::{DatabaseConfig, NvrDatabase};
use crate::user::{self, UserInfo};

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

fn user(username: &str, password: &str) -> UserInfo {
    let now = Utc::now();
    UserInfo {
        username: username.to_string(),
        password_hash: user::hash_password(password).unwrap(),
        metadata: HashMap::new(),
        create_time: now,
        update_time: now,
    }
}

#[test]
fn hash_and_verify_round_trip() {
    let hash = user::hash_password("secret").unwrap();
    assert!(user::verify_password("secret", &hash));
    assert!(!user::verify_password("wrong", &hash));
    assert!(!user::verify_password("secret", "not-a-hash"));
}

#[tokio::test]
async fn update_replaces_password_hash() {
    let conn = test_conn().await;
    user::insert(&user("alice", "old"), &conn).await.unwrap();

    let mut updated = user::get_by_username("alice", &conn)
        .await
        .unwrap()
        .unwrap();
    updated.password_hash = user::hash_password("new").unwrap();
    user::update(&updated, &conn).await.unwrap();

    let found = user::get_by_username("alice", &conn)
        .await
        .unwrap()
        .unwrap();
    assert!(user::verify_password("new", &found.password_hash));
    assert!(!user::verify_password("old", &found.password_hash));
}

#[tokio::test]
async fn delete_and_list() {
    let conn = test_conn().await;
    user::insert(&user("alice", "a"), &conn).await.unwrap();
    user::insert(&user("bob", "b"), &conn).await.unwrap();

    let mut names: Vec<String> = user::list(&conn)
        .await
        .unwrap()
        .into_iter()
        .map(|u| u.username)
        .collect();
    names.sort();
    assert_eq!(names, vec!["alice", "bob"]);

    user::delete("alice", &conn).await.unwrap();
    assert!(!user::exists("alice", &conn).await.unwrap());
    assert!(user::exists("bob", &conn).await.unwrap());
}
