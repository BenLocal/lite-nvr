// db_test is a child module of `db` (this file), so `super` == db.
use super::{DatabaseConfig, NvrDatabase};

#[tokio::test]
async fn test_connect() {
    let config = DatabaseConfig { url: ":memory:" };
    let db = NvrDatabase::new(&config).await.unwrap();
    let conn = db.connect();
    assert!(conn.is_ok());
}

/// Writers on separate connections (one per unit of work, like the app does)
/// queue for the WAL write lock instead of failing with `database is locked`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writers_wait_for_the_write_lock() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("concurrent.db");
    let db =
        std::sync::Arc::new(NvrDatabase::new(&DatabaseConfig::new(path.to_str().unwrap())).await?);
    db.connect()?
        .execute("CREATE TABLE item (id TEXT PRIMARY KEY)", ())
        .await?;

    let writers = (0..8).map(|w| {
        let db = db.clone();
        tokio::spawn(async move {
            for i in 0..25 {
                let id = format!("{w}-{i}");
                db.connect()?
                    .execute("INSERT INTO item (id) VALUES (?1)", [id.as_str()])
                    .await?;
                db.connect()?
                    .execute("DELETE FROM item WHERE id = ?1", [id.as_str()])
                    .await?;
            }
            anyhow::Ok(())
        })
    });
    for writer in futures::future::join_all(writers).await {
        writer??;
    }
    Ok(())
}
