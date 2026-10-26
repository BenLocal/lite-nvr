use std::io::Write;

use super::*;

#[test]
fn model_checks_run_before_init() {
    let dir = tempfile::tempdir().unwrap();
    let err = check_model(&dir.path().join("missing.rkllm")).unwrap_err();
    assert!(err.to_string().contains("missing.rkllm"), "{err:#}");

    let wrong = dir.path().join("model.gguf");
    std::fs::write(&wrong, b"x").unwrap();
    assert!(
        check_model(&wrong)
            .unwrap_err()
            .to_string()
            .contains(".rkllm")
    );

    let empty = dir.path().join("empty.rkllm");
    std::fs::File::create(&empty).unwrap();
    assert!(check_model(&empty).is_err());

    let ok = dir.path().join("ok.rkllm");
    std::fs::File::create(&ok)
        .unwrap()
        .write_all(b"model")
        .unwrap();
    check_model(&ok).unwrap();
}

#[tokio::test]
async fn a_bad_model_fails_the_engine_without_loading_the_runtime() {
    let cfg = LlmConfig::new("/nonexistent/model.rkllm", crate::Template::Qwen);
    let engine = crate::Engine::spawn(cfg);
    let err = engine.wait_ready().await.unwrap_err();
    assert!(err.to_string().contains("model.rkllm"), "{err:#}");
}

/// On a Rockchip host: `NVR_RKLLM_TEST_MODEL=/userdata/.../x.rkllm` (and
/// `NVR_RKLLM_TEST_LIB` when librkllmrt.so is not on `LD_LIBRARY_PATH`) loads
/// the model and answers one question.
#[tokio::test]
async fn rockchip_model_answers() {
    use futures::StreamExt;
    let Ok(model) = std::env::var("NVR_RKLLM_TEST_MODEL") else {
        return;
    };
    let mut cfg = LlmConfig::new(model, crate::Template::Qwen);
    if let Ok(lib) = std::env::var("NVR_RKLLM_TEST_LIB") {
        cfg.lib = lib;
    }
    let engine = crate::process::process_test::spawn_config(cfg, "real");
    engine.wait_ready().await.unwrap();
    let req = crate::ChatRequest {
        messages: vec![crate::Message::user("用一句话介绍你自己")],
        max_tokens: Some(64),
        ..Default::default()
    };
    let deltas: Vec<_> = engine.generate(req).unwrap().collect().await;
    let text: String = deltas
        .iter()
        .filter_map(|d| match d {
            crate::Delta::Content(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert!(!text.is_empty());
    assert!(matches!(deltas.last(), Some(crate::Delta::Done { .. })));
}
