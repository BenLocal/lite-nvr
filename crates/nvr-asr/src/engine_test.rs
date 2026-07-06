use super::strip_punct;

#[test]
fn strip_punct_removes_cjk_and_ascii() {
    // Chinese sentence punctuation.
    assert_eq!(
        strip_punct("派饭时间，早上9点至下午5点。"),
        "派饭时间早上9点至下午5点"
    );
    assert_eq!(strip_punct("你好、世界；测试：完成！"), "你好世界测试完成");
    // ASCII punctuation (spaces are kept).
    assert_eq!(strip_punct("hello, world!"), "hello world");
}

#[test]
fn strip_punct_keeps_digits_and_letters() {
    assert_eq!(strip_punct("9点abc九"), "9点abc九");
}

// Requires real models on disk; run manually with:
//   SHERPA_ONNX_LIB_DIR=... cargo test -p nvr-asr -- --ignored shared_models
#[test]
#[ignore = "needs ~1.3GB models under third_party/asr-models"]
fn shared_models_drive_two_independent_engines() {
    use crate::{AsrConfig, AsrEngine, AsrModels};
    let base = std::path::Path::new("third_party/asr-models");
    let sv = base.join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17");
    let cfg = AsrConfig::new(
        sv.join("model.int8.onnx"),
        sv.join("tokens.txt"),
        base.join("silero_vad.onnx"),
    );
    let models = AsrModels::load(cfg).expect("load models");
    let mut a = AsrEngine::new(models.clone()).expect("engine a");
    let mut b = AsrEngine::new(models.clone()).expect("engine b");
    let silence = vec![0.0f32; 16_000];
    let _ = a.accept(&silence);
    let _ = b.accept(&silence);
    assert!(std::sync::Arc::strong_count(&models) >= 3); // models + a + b
}
