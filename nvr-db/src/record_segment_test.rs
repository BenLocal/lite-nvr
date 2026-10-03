use super::*;

#[tokio::test]
async fn retention_compares_rfc3339_times_as_dates() {
    let db = turso::Builder::new_local(":memory:").build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute_batch(include_str!("../migrations/20260317_record_segment.sql"))
        .await
        .unwrap();
    let cutoff = Utc::now() - chrono::Duration::days(1);
    let expired = (cutoff - chrono::Duration::seconds(2)).to_rfc3339();
    let fresh = (cutoff + chrono::Duration::hours(1)).to_rfc3339();
    conn.execute("INSERT INTO record_segments (id,file_path,create_time,update_time) VALUES ('expired','expired',?1,?1),('fresh','fresh',?2,?2)",(expired.as_str(),fresh.as_str())).await.unwrap();
    let expired_records = list_older_than_days(1, &conn).await.unwrap();
    assert_eq!(expired_records.len(), 1);
    assert_eq!(expired_records[0].id, "expired");
}

#[tokio::test]
async fn batch_lookup_and_delete_escape_ids_and_ignore_duplicates() {
    let db = turso::Builder::new_local(":memory:").build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute_batch(include_str!("../migrations/20260317_record_segment.sql"))
        .await
        .unwrap();
    let now = Utc::now().to_rfc3339();
    conn.execute("INSERT INTO record_segments (id,file_path,create_time,update_time) VALUES (?1,'quoted',?2,?2),('keep','keep',?2,?2)",("quoted'; DELETE FROM record_segments; --",now.as_str())).await.unwrap();
    let ids = vec![
        "quoted'; DELETE FROM record_segments; --".into(),
        "missing".into(),
        "quoted'; DELETE FROM record_segments; --".into(),
    ];
    assert_eq!(list_by_ids(&ids, &conn).await.unwrap().len(), 1);
    delete_ids(&ids, &conn).await.unwrap();
    assert_eq!(count(&conn).await.unwrap(), 1);
    assert!(get("keep", &conn).await.unwrap().is_some());
    assert!(list_by_ids(&[], &conn).await.unwrap().is_empty());
    delete_ids(&[], &conn).await.unwrap();
}
