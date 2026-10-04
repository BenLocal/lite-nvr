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

#[tokio::test]
async fn transport_filters_devices_before_limiting_and_keeps_retry_semantics() {
    let db = turso::Builder::new_local(":memory:").build().await.unwrap();
    let conn = db.connect().unwrap();
    conn.execute_batch(include_str!("../migrations/20260317_record_segment.sql"))
        .await
        .unwrap();
    conn.execute_batch(include_str!("../migrations/20260702_transport.sql"))
        .await
        .unwrap();
    let quoted = "cam'; DELETE FROM record_segments; --";
    conn.execute("INSERT INTO record_segments (id,file_path,stream,start_time) VALUES ('other','other','other',1),('selected','selected',?1,2),('retry','retry',?1,3),('exhausted','exhausted',?1,4),('done','done',?1,5)", [quoted]).await.unwrap();
    conn.execute_batch("INSERT INTO transport_jobs (id,segment_id,target_id,status,attempts) VALUES ('j1','retry','target',2,4),('j2','exhausted','target',2,5),('j3','done','target',1,1)").await.unwrap();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE record_segments SET create_time=?1, update_time=?1",
        [now.as_str()],
    )
    .await
    .unwrap();
    let selected = vec![quoted.to_string()];
    let records = list_needing_transport("target", 5, 1, Some(&selected), &conn)
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "selected");
    let records = list_needing_transport("target", 5, 20, Some(&selected), &conn)
        .await
        .unwrap();
    assert_eq!(
        records.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec!["selected", "retry"]
    );
    assert!(
        list_needing_transport("target", 5, 20, Some(&[]), &conn)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        list_needing_transport("target", 5, 20, None, &conn)
            .await
            .unwrap()
            .len(),
        3
    );
    assert_eq!(count(&conn).await.unwrap(), 5);
}
