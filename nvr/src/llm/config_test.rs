use std::collections::HashMap;

use super::*;

fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = vars
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |name| map.get(name).cloned()
}

#[test]
fn llm_is_off_without_a_model() {
    assert!(from_lookup(lookup(&[])).unwrap().is_none());
    assert!(
        from_lookup(lookup(&[("NVR_LLM_MODEL", "  ")]))
            .unwrap()
            .is_none()
    );
}

#[test]
fn defaults_and_overrides() {
    let cfg = from_lookup(lookup(&[("NVR_LLM_MODEL", "/m/q.rkllm")]))
        .unwrap()
        .unwrap();
    assert_eq!(cfg.template, nvr_rkllm::Template::Qwen);
    assert_eq!(cfg.lib, "librkllmrt.so");
    assert_eq!(cfg.stop_token_ids, nvr_rkllm::DEFAULT_STOP_TOKEN_IDS);

    let cfg = from_lookup(lookup(&[
        ("NVR_LLM_MODEL", "/m/r1.rkllm"),
        ("NVR_LLM_TEMPLATE", "deepseek-r1"),
        ("NVR_LLM_LIB", "/opt/lib/librkllmrt.so"),
        ("NVR_LLM_MAX_CONTEXT", "2048"),
        ("NVR_LLM_MAX_NEW_TOKENS", "256"),
        ("NVR_LLM_STOP_TOKENS", "1, 2"),
        ("NVR_LLM_IGNORE_EOS", "1"),
    ]))
    .unwrap()
    .unwrap();
    assert_eq!(cfg.template, nvr_rkllm::Template::DeepseekR1);
    assert_eq!(cfg.lib, "/opt/lib/librkllmrt.so");
    assert_eq!((cfg.max_context_len, cfg.max_new_tokens), (2048, 256));
    assert_eq!(cfg.stop_token_ids, vec![1, 2]);
    assert!(cfg.ignore_eos);
}

#[test]
fn bad_values_name_the_variable() {
    for (name, value) in [
        ("NVR_LLM_TEMPLATE", "llama"),
        ("NVR_LLM_MAX_CONTEXT", "big"),
        ("NVR_LLM_STOP_TOKENS", "1,x"),
    ] {
        let err =
            from_lookup(lookup(&[("NVR_LLM_MODEL", "/m/q.rkllm"), (name, value)])).unwrap_err();
        assert!(format!("{err:#}").contains(name), "{err:#}");
    }
}
