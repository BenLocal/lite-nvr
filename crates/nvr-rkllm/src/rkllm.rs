//! [`Backend`] over the real RKLLM runtime.

use std::ffi::{CStr, CString, c_int, c_void};
use std::os::unix::ffi::OsStrExt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use anyhow::{Context, Result};

use crate::LlmConfig;
use crate::engine::{Backend, Run, Step};
use crate::ffi::{self, LLMHandle, RkllmLib};

/// Chat defaults (Qwen's recommendation for non-thinking replies); requests
/// may override them.
const TOP_K: i32 = 20;
const TOP_P: f32 = 0.8;
const TEMPERATURE: f32 = 0.7;
const REPEAT_PENALTY: f32 = 1.05;

pub(crate) struct RkllmBackend {
    lib: RkllmLib,
    handle: LLMHandle,
    defaults: ffi::RKLLMParam,
    // `rkllm_init` was given this pointer; keep it alive with the handle.
    _model_path: CString,
}

/// Everything `rkllm_init` would trip over, checked up front: a failed init
/// may double free inside runtime 1.3.0 and take the process down.
pub(crate) fn check_model(path: &Path) -> Result<()> {
    if path.extension().and_then(|e| e.to_str()) != Some("rkllm") {
        anyhow::bail!("{} is not a .rkllm model", path.display());
    }
    let meta = std::fs::metadata(path).with_context(|| format!("model {}", path.display()))?;
    if !meta.is_file() || meta.len() == 0 {
        anyhow::bail!("model {} is not a non-empty file", path.display());
    }
    std::fs::File::open(path).with_context(|| format!("open model {}", path.display()))?;
    Ok(())
}

impl RkllmBackend {
    pub(crate) fn load(cfg: &LlmConfig) -> Result<Self> {
        check_model(&cfg.model)?;
        let lib = ffi::load(&cfg.lib)?;
        let model_path = CString::new(cfg.model.as_os_str().as_bytes())?;

        // SAFETY: plain value-returning constructor.
        let mut param = unsafe { lib.rkllm_createDefaultParam() };
        param.model_path = model_path.as_ptr();
        param.max_context_len = cfg.max_context_len;
        param.max_new_tokens = cfg.max_new_tokens;
        param.top_k = TOP_K;
        param.top_p = TOP_P;
        param.temperature = TEMPERATURE;
        param.repeat_penalty = REPEAT_PENALTY;
        param.skip_special_token = true;
        param.ignore_eos_token = cfg.ignore_eos;
        param.is_async = false;
        param.extend_param.base_domain_id = 0;
        param.extend_param.embed_flash = 1;
        param.extend_param.enabled_cpus_num = 4;
        param.extend_param.enabled_cpus_mask = ffi::BIG_CORES_MASK;

        let mut callback = ffi::RKLLMCallback {
            result_callback: Some(on_result),
            ..Default::default()
        };
        let mut handle: LLMHandle = std::ptr::null_mut();
        log::info!(
            "rkllm: loading {} (max_context_len {})",
            cfg.model.display(),
            cfg.max_context_len
        );
        // SAFETY: param, callback and the model path outlive the call.
        let ret = unsafe { lib.rkllm_init(&mut handle, &mut param, &mut callback) };
        if ret != 0 || handle.is_null() {
            anyhow::bail!("rkllm_init failed ({ret})");
        }
        // Prompts are rendered by `Template`; an empty runtime template keeps
        // them verbatim.
        // SAFETY: valid handle and NUL-terminated strings.
        let ret = unsafe {
            lib.rkllm_set_chat_template(handle, c"".as_ptr(), c"".as_ptr(), c"".as_ptr())
        };
        if ret != 0 {
            anyhow::bail!("rkllm_set_chat_template failed ({ret})");
        }
        log::info!("rkllm: model ready");
        Ok(Self {
            lib,
            handle,
            defaults: param,
            _model_path: model_path,
        })
    }
}

struct RunCtx<'a> {
    on_token: &'a mut dyn FnMut(i32, &str) -> Step,
    abort: unsafe extern "C" fn(LLMHandle) -> c_int,
    handle: LLMHandle,
    aborted: bool,
    failed: bool,
    prompt_tokens: u32,
}

unsafe extern "C" fn on_result(
    result: *mut ffi::RKLLMResult,
    userdata: *mut c_void,
    state: ffi::LLMCallState,
) -> c_int {
    // A panic must not unwind into the runtime.
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `run` passes a live `RunCtx` as the per-run userdata, which
        // the runtime prefers over the init-time one.
        let ctx = unsafe { &mut *(userdata as *mut RunCtx<'_>) };
        // SAFETY: the runtime passes a valid result for the callback's duration.
        let result = unsafe { result.as_ref() };
        match state {
            ffi::RKLLM_RUN_NORMAL | ffi::RKLLM_RUN_WAITING if !ctx.aborted => {
                let Some(result) = result else { return };
                let text = if state == ffi::RKLLM_RUN_NORMAL && !result.text.is_null() {
                    // SAFETY: NUL-terminated text owned by the runtime.
                    unsafe { CStr::from_ptr(result.text) }.to_string_lossy()
                } else {
                    // Waiting for the rest of a UTF-8 character.
                    "".into()
                };
                if (ctx.on_token)(result.token_id, &text) == Step::Stop {
                    ctx.aborted = true;
                    // SAFETY: valid handle; aborting from the callback ends the run.
                    unsafe { (ctx.abort)(ctx.handle) };
                }
            }
            ffi::RKLLM_RUN_FINISH => {
                if let Some(result) = result {
                    ctx.prompt_tokens = u32::try_from(result.perf.prefill_tokens).unwrap_or(0);
                }
            }
            ffi::RKLLM_RUN_ERROR => ctx.failed = true,
            _ => {}
        }
    }));
    0
}

impl Backend for RkllmBackend {
    fn run(&mut self, run: Run<'_>, on_token: &mut dyn FnMut(i32, &str) -> Step) -> Result<u32> {
        let prompt = CString::new(run.prompt).context("prompt contains a NUL byte")?;
        let mut input = ffi::RKLLMInput {
            role: c"user".as_ptr(),
            enable_thinking: false,
            input_type: ffi::RKLLM_INPUT_PROMPT,
            __bindgen_anon_1: ffi::RKLLMInput__bindgen_ty_1 {
                prompt_input: prompt.as_ptr(),
            },
        };
        let d = &self.defaults;
        let s = run.sampling;
        let mut sampling = ffi::RKLLMSamplingParam {
            top_k: s.top_k.unwrap_or(d.top_k),
            top_p: s.top_p.unwrap_or(d.top_p),
            temperature: s.temperature.unwrap_or(d.temperature),
            repeat_penalty: s.repetition_penalty.unwrap_or(d.repeat_penalty),
            frequency_penalty: d.frequency_penalty,
            presence_penalty: d.presence_penalty,
            mirostat: d.mirostat,
            mirostat_tau: d.mirostat_tau,
            mirostat_eta: d.mirostat_eta,
        };
        let mut infer = ffi::RKLLMInferParam {
            mode: ffi::RKLLM_INFER_GENERATE,
            lora_params: std::ptr::null_mut(),
            prompt_cache_params: std::ptr::null_mut(),
            sampling_params: &mut sampling,
            keep_history: 0,
            max_new_tokens: run.max_new_tokens,
        };
        let mut ctx = RunCtx {
            on_token,
            abort: self.lib.rkllm_abort,
            handle: self.handle,
            aborted: false,
            failed: false,
            prompt_tokens: 0,
        };
        // SAFETY: all pointers outlive this synchronous call; the callback
        // receives `ctx` as its userdata.
        let ret = unsafe {
            self.lib.rkllm_run(
                self.handle,
                &mut input,
                &mut infer,
                &mut ctx as *mut RunCtx<'_> as *mut c_void,
            )
        };
        if ctx.aborted {
            return Ok(ctx.prompt_tokens);
        }
        if ret != 0 || ctx.failed {
            anyhow::bail!("rkllm_run failed ({ret})");
        }
        Ok(ctx.prompt_tokens)
    }
}

// No Drop: `rkllm_destroy` can hang in an NPU ioctl, so the handle is left for
// the kernel to reclaim at process exit.

#[cfg(test)]
#[path = "rkllm_test.rs"]
mod rkllm_test;
