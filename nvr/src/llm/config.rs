//! `NVR_LLM_*` environment configuration.

use anyhow::{Context, Result};
use nvr_rkllm::LlmConfig;

/// Reads the LLM config through `get` (the environment in production).
/// `Ok(None)` when `NVR_LLM_MODEL` is unset or empty: the LLM is off.
pub(crate) fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Option<LlmConfig>> {
    let var = |name: &str| {
        get(name)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let Some(model) = var("NVR_LLM_MODEL") else {
        return Ok(None);
    };
    let template = match var("NVR_LLM_TEMPLATE") {
        Some(t) => t.parse().context("NVR_LLM_TEMPLATE")?,
        None => nvr_rkllm::Template::Qwen,
    };
    let mut cfg = LlmConfig::new(model, template);
    if let Some(lib) = var("NVR_LLM_LIB") {
        cfg.lib = lib;
    }
    if let Some(n) = var("NVR_LLM_MAX_CONTEXT") {
        cfg.max_context_len = n.parse().context("NVR_LLM_MAX_CONTEXT")?;
    }
    if let Some(n) = var("NVR_LLM_MAX_NEW_TOKENS") {
        cfg.max_new_tokens = n.parse().context("NVR_LLM_MAX_NEW_TOKENS")?;
    }
    if let Some(ids) = var("NVR_LLM_STOP_TOKENS") {
        cfg.stop_token_ids = ids
            .split(',')
            .map(|id| id.trim().parse())
            .collect::<Result<_, _>>()
            .context("NVR_LLM_STOP_TOKENS")?;
    }
    if let Some(flag) = var("NVR_LLM_IGNORE_EOS") {
        cfg.ignore_eos = matches!(flag.as_str(), "1" | "true");
    }
    Ok(Some(cfg))
}

#[cfg(test)]
#[path = "config_test.rs"]
mod config_test;
