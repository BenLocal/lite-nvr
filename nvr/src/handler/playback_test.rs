use super::*;
use axum::{body::Body, http::Request};
use tower::ServiceExt;

#[tokio::test]
async fn deletion_endpoints_keep_failed_files_and_remove_successful_rows() {
    let _db = crate::auth::auth_test::ensure_test_db().await;
    let conn = app_db_conn().unwrap();
    conn.execute_batch(include_str!(
        "../../../nvr-db/migrations/20260317_record_segment.sql"
    ))
    .await
    .unwrap();
    for (index, route) in [
        "/segment/bad-0/delete",
        "/segments/delete",
        "/device/delete-regression-2/segments/delete",
    ]
    .iter()
    .enumerate()
    {
        let dir =
            std::env::temp_dir().join(format!("nvr-playback-delete-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir(&dir).await.unwrap();
        let good = dir.join("good.ts");
        tokio::fs::write(&good, b"recording").await.unwrap();
        let bad_id = format!("bad-{index}");
        let good_id = format!("good-{index}");
        let stream = format!("delete-regression-{index}");
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO record_segments (id,file_path,stream,create_time,update_time) VALUES (?1,?2,?3,?4,?4),(?5,?6,?3,?4,?4)", (bad_id.as_str(),dir.to_string_lossy().as_ref(),stream.as_str(),now.as_str(),good_id.as_str(),good.to_string_lossy().as_ref())).await.unwrap();
        let body = serde_json::json!({"ids":[bad_id,good_id]}).to_string();
        let response = playback_router()
            .oneshot(
                Request::post(*route)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            nvr_db::record_segment::get(&bad_id, &conn)
                .await
                .unwrap()
                .is_some()
        );
        if index > 0 {
            assert!(
                nvr_db::record_segment::get(&good_id, &conn)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(!good.exists());
        } else {
            tokio::fs::remove_file(&good).await.unwrap();
        }
        nvr_db::record_segment::delete_by_stream(&stream, &conn)
            .await
            .unwrap();
        tokio::fs::remove_dir(&dir).await.unwrap();
    }
}
