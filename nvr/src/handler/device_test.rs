use super::device_router;
use crate::detect::hub::DetectHub;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use serde_json::json;
use std::path::PathBuf;
use tower::ServiceExt;

fn request(path: &str, body: serde_json::Value) -> Request<Body> {
    Request::post(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn listed_device(hub: &'static DetectHub, id: &str) -> nvr_db::device::DeviceInfo {
    let response = device_router(hub)
        .oneshot(
            Request::get("/list")
                .body(Body::empty())
                .expect("list request"),
        )
        .await
        .expect("list response");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("list body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("list json");
    let item = json["data"]
        .as_array()
        .expect("device list")
        .iter()
        .find(|item| item["id"] == id)
        .expect("listed device")
        .clone();
    serde_json::from_value(item).expect("device payload")
}

#[tokio::test]
async fn add_and_update_endpoints_persist_detection_config() {
    let _db = crate::auth::auth_test::ensure_test_db().await;
    let conn = crate::db::app_db_conn().expect("test db");
    nvr_db::device::delete("detect-persist", &conn)
        .await
        .expect("clean test device");
    let hub = Box::leak(Box::new(DetectHub::new_for_test(
        vec![],
        PathBuf::new(),
        500,
    )));

    let add = json!({
        "id": "detect-persist",
        "name": "Detection persistence",
        "input_type": "gb28181",
        "input_value": r#"{"device_id":"platform","channel_id":"channel"}"#,
        "config": {
            "detect": {
                "enabled": true,
                "models": [" known ", "retired", "known"],
                "sample_every_ms": 1_234,
                "min_confidence": 0.55
            }
        }
    });
    let response = device_router(hub)
        .oneshot(request("/add", add))
        .await
        .expect("add response");
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let saved = listed_device(hub, "detect-persist").await;
    let detect = saved.config.detect.expect("added detection config");
    assert!(detect.enabled);
    assert_eq!(detect.models, vec!["known", "retired"]);
    assert_eq!(detect.sample_every_ms, 1_234);
    assert!((detect.min_confidence - 0.55).abs() < f32::EPSILON);

    let update = json!({
        "name": "Detection persistence updated",
        "input_type": "gb28181",
        "input_value": r#"{"device_id":"platform","channel_id":"channel"}"#,
        "config": {
            "detect": {
                "enabled": false,
                "models": ["retired"],
                "sample_every_ms": 3_600_000,
                "min_confidence": 0.8
            }
        }
    });
    let response = device_router(hub)
        .oneshot(request("/update/detect-persist", update))
        .await
        .expect("update response");
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let saved = listed_device(hub, "detect-persist").await;
    let detect = saved.config.detect.expect("updated detection config");
    assert!(!detect.enabled);
    assert_eq!(detect.models, vec!["retired"]);
    assert_eq!(detect.sample_every_ms, 3_600_000);
    assert!((detect.min_confidence - 0.8).abs() < f32::EPSILON);

    nvr_db::device::delete("detect-persist", &conn)
        .await
        .expect("remove test device");
}

#[tokio::test]
async fn duplicate_add_preserves_existing_device() {
    let _db = crate::auth::auth_test::ensure_test_db().await;
    let hub = Box::leak(Box::new(DetectHub::new_for_test(
        vec![],
        PathBuf::new(),
        500,
    )));
    for (channel, expected) in [
        ("first-camera", axum::http::StatusCode::OK),
        (
            "second-camera",
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        ),
    ] {
        let payload = json!({"name":"review-duplicate", "input_type":"gb28181", "input_value":json!({"device_id":"platform","channel_id":channel}).to_string()});
        let response = device_router(hub)
            .oneshot(request("/add", payload))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    let conn = crate::db::app_db_conn().unwrap();
    let devices = nvr_db::device::list(&conn).await.unwrap();
    let matches: Vec<_> = devices
        .iter()
        .filter(|d| d.name == "review-duplicate")
        .collect();
    assert_eq!(matches.len(), 1);
    assert!(matches[0].input_value.contains("first-camera"));
}

#[tokio::test]
async fn invalid_update_preserves_existing_config() {
    let _db = crate::auth::auth_test::ensure_test_db().await;
    let hub = Box::leak(Box::new(DetectHub::new_for_test(
        vec![],
        PathBuf::new(),
        500,
    )));
    let good = json!({"id":"review-invalid", "name":"valid", "input_type":"gb28181", "input_value":r#"{"device_id":"platform","channel_id":"camera"}"#});
    let response = device_router(hub)
        .oneshot(request("/add", good))
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bad = json!({"name":"broken", "input_type":"gb28181", "input_value":"invalid-json"});
    let response = device_router(hub)
        .oneshot(request("/update/review-invalid", bad))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    let saved = listed_device(hub, "review-invalid").await;
    assert!(saved.input_value.contains("camera"));
    assert_eq!(saved.name, "valid");
}

#[tokio::test]
async fn concurrent_add_has_one_winner() {
    let _db = crate::auth::auth_test::ensure_test_db().await;
    let hub = Box::leak(Box::new(DetectHub::new_for_test(
        vec![],
        PathBuf::new(),
        500,
    )));
    let payload = json!({"id":"concurrent-add", "name":"concurrent add", "input_type":"gb28181", "input_value":r#"{"device_id":"platform","channel_id":"camera"}"#});
    let (first, second) = tokio::join!(
        device_router(hub).oneshot(request("/add", payload.clone())),
        device_router(hub).oneshot(request("/add", payload)),
    );
    let statuses = [first.unwrap().status(), second.unwrap().status()];
    assert_eq!(
        statuses
            .iter()
            .filter(|&&s| s == axum::http::StatusCode::OK)
            .count(),
        1
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|&&s| s == axum::http::StatusCode::INTERNAL_SERVER_ERROR)
            .count(),
        1
    );
    let conn = crate::db::app_db_conn().unwrap();
    nvr_db::device::delete("concurrent-add", &conn)
        .await
        .unwrap();
}

#[tokio::test]
async fn invalid_add_never_persists_a_device() {
    let _db = crate::auth::auth_test::ensure_test_db().await;
    let hub = Box::leak(Box::new(DetectHub::new_for_test(
        vec![],
        PathBuf::new(),
        500,
    )));
    let conn = crate::db::app_db_conn().unwrap();
    for (id, input_type, input_value) in [
        ("invalid-add-json", "gb28181", "invalid-json"),
        ("invalid-add-type", "unsupported", "value"),
    ] {
        let payload = json!({"id":id, "name":"invalid add", "input_type":input_type, "input_value":input_value});
        let response = device_router(hub)
            .oneshot(request("/add", payload))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        );
        assert!(nvr_db::device::get(id, &conn).await.unwrap().is_none());
    }
}
