use super::*;

#[test]
fn health_reports_disabled_and_misconfigured() {
    let (status, body) = health_body(&Llm::Disabled);
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "disabled");

    let (status, body) = health_body(&Llm::Invalid("NVR_LLM_TEMPLATE: bad".into()));
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "failed");
    assert_eq!(body["reason"], "NVR_LLM_TEMPLATE: bad");
}

#[tokio::test]
async fn health_reports_a_failed_load() {
    let engine = Engine::spawn(nvr_rkllm::LlmConfig::new(
        "/nonexistent/q.rkllm",
        nvr_rkllm::Template::Qwen,
    ));
    assert!(engine.wait_ready().await.is_err());
    let llm = Llm::Enabled {
        engine,
        model: "q".into(),
    };
    let (status, body) = health_body(&llm);
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["status"], "failed");
    assert_eq!(body["model"], "q");
    assert!(
        body["reason"]
            .as_str()
            .unwrap()
            .contains("/nonexistent/q.rkllm")
    );
}
