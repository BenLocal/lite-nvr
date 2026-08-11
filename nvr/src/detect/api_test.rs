use super::{StartBody, detect_router};
use crate::detect::hub::DetectHub;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use tower::ServiceExt;

#[test]
fn start_body_defaults_models_to_none() {
    // Empty body → run all configured models.
    let b: StartBody = serde_json::from_str("{}").unwrap();
    assert!(b.models.is_none());

    let b: StartBody = serde_json::from_str(r#"{"models":["yolov8n"]}"#).unwrap();
    assert_eq!(b.models.unwrap(), vec!["yolov8n".to_string()]);
}

#[tokio::test]
async fn stop_endpoint_prevents_pending_auto_start_from_starting_later() {
    let hub = Box::leak(Box::new(DetectHub::new_for_test(
        vec![],
        PathBuf::new(),
        500,
    )));
    let (generation, pending) = hub.begin_auto_start("cam1");
    let app = detect_router(hub);

    let response = app
        .oneshot(
            Request::post("/cam1/stop")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(pending.is_cancelled());
    assert!(
        hub.register_auto_start("cam1", generation, CancellationToken::new())
            .is_none()
    );
}

#[tokio::test]
async fn leased_stop_endpoint_does_not_stop_a_replacement_tap() {
    let hub = Box::leak(Box::new(DetectHub::new_for_test(
        vec![],
        PathBuf::new(),
        500,
    )));
    let old_epoch = hub
        .register("cam1", CancellationToken::new())
        .expect("old tap");
    assert!(hub.unregister("cam1"));
    let replacement = CancellationToken::new();
    hub.register("cam1", replacement.clone())
        .expect("replacement tap");

    let response = detect_router(hub)
        .oneshot(
            Request::post("/cam1/stop")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "lease": old_epoch.raw().to_string() }).to_string(),
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(hub.is_running("cam1"));
    assert!(!replacement.is_cancelled());
    hub.unregister("cam1");
}

#[tokio::test]
async fn capabilities_endpoint_is_the_detection_input_contract() {
    let hub = Box::leak(Box::new(DetectHub::new_for_test(
        vec![],
        PathBuf::new(),
        500,
    )));
    let response = detect_router(hub)
        .oneshot(
            Request::get("/capabilities")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    let json: serde_json::Value = serde_json::from_slice(&body).expect("capabilities json");
    assert_eq!(json["models"], serde_json::json!([]));
    assert_eq!(
        json["supported_input_types"],
        serde_json::json!(["net", "rtsp", "rtmp", "file", "v4l2", "x11grab", "lavfi"])
    );
    assert_eq!(json["max_sample_interval_ms"], 3_600_000);
    assert_eq!(json["max_model_count"], 32);
    assert_eq!(json["max_model_name_chars"], 128);
}
