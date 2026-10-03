use super::*;

#[tokio::test]
async fn failed_file_removal_preserves_database_record() {
    let db = turso::Builder::new_local(":memory:").build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute_batch(include_str!(
        "../../nvr-db/migrations/20260317_record_segment.sql"
    ))
    .await
    .unwrap();
    let dir = std::env::temp_dir().join(format!("nvr-cleanup-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir(&dir).await.unwrap();
    let path = dir.to_string_lossy().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO record_segments (id,file_path,create_time,update_time) VALUES (?1,?2,?3,?3)",
        ("failed", path.as_str(), now.as_str()),
    )
    .await
    .unwrap();
    let segment = record_segment::get("failed", &conn).await.unwrap().unwrap();
    let result = remove_segments(&[segment], &conn, None).await.unwrap();
    assert_eq!(result.removed, 0);
    assert_eq!(result.freed, 0);
    assert_eq!(result.failed, 1);
    let kept = record_segment::get("failed", &conn)
        .await
        .unwrap()
        .is_some();
    tokio::fs::remove_dir(&dir).await.unwrap();
    assert!(
        kept,
        "file deletion failed but the DB row was still removed"
    );
}

#[tokio::test]
async fn size_cleanup_skips_failed_files_and_counts_only_successes() {
    let db = turso::Builder::new_local(":memory:").build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute_batch(include_str!(
        "../../nvr-db/migrations/20260317_record_segment.sql"
    ))
    .await
    .unwrap();
    let dir = std::env::temp_dir().join(format!("nvr-cleanup-batch-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir(&dir).await.unwrap();
    let first = dir.join("first.ts");
    let second = dir.join("second.ts");
    tokio::fs::write(&first, vec![0u8; 10]).await.unwrap();
    tokio::fs::write(&second, vec![0u8; 20]).await.unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    for (id, path, size) in [
        ("failed", dir.clone(), 100),
        ("first", first.clone(), 10),
        ("second", second.clone(), 20),
    ] {
        conn.execute("INSERT INTO record_segments (id,file_path,file_size,create_time,update_time) VALUES (?1,?2,?3,?4,?4)", (id,path.to_string_lossy().as_ref(),size,now.as_str())).await.unwrap();
    }
    let ids = vec!["failed".into(), "first".into(), "second".into()];
    let mut records = record_segment::list_by_ids(&ids, &conn).await.unwrap();
    records.sort_by_key(|r| ids.iter().position(|id| id == &r.id).unwrap());
    let result = remove_segments(&records, &conn, Some(15)).await.unwrap();
    assert_eq!((result.removed, result.freed, result.failed), (2, 30, 1));
    assert!(
        record_segment::get("failed", &conn)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(record_segment::count(&conn).await.unwrap(), 1);
    assert!(!first.exists() && !second.exists());
    tokio::fs::remove_dir(&dir).await.unwrap();
}

#[tokio::test]
async fn already_missing_file_can_remove_its_stale_record() {
    let db = turso::Builder::new_local(":memory:").build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute_batch(include_str!(
        "../../nvr-db/migrations/20260317_record_segment.sql"
    ))
    .await
    .unwrap();
    let path = std::env::temp_dir().join(format!("missing-nvr-file-{}", uuid::Uuid::new_v4()));
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute("INSERT INTO record_segments (id,file_path,create_time,update_time) VALUES ('missing',?1,?2,?2)",(path.to_string_lossy().as_ref(),now.as_str())).await.unwrap();
    let records = record_segment::list(&conn).await.unwrap();
    let result = remove_segments(&records, &conn, None).await.unwrap();
    assert_eq!((result.removed, result.failed), (1, 0));
    assert_eq!(record_segment::count(&conn).await.unwrap(), 0);
}
