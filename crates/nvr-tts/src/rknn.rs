//! Minimal dlopen binding to the Rockchip NPU runtime (`librknnrt.so`) for
//! running one single-output `.rknn` model.
//!
//! Hand-written rather than generated: `rknn_api.h` carries a proprietary
//! Rockchip notice, so it is not vendored. Only the calls and structs used
//! here are declared; their layouts are pinned below with values from
//! compiling `rknn_api.h` (runtime 2.x) with gcc for LP64 targets.

use std::ffi::{c_int, c_void};
use std::path::Path;

use anyhow::{Context, Result};
use libloading::Library;

type Ctx = u64;

const QUERY_IN_OUT_NUM: c_int = 0;
const TENSOR_FLOAT32: c_int = 0;
const FORMAT_UNDEFINED: c_int = 3;

/// `rknn_core_mask` value that pins a model to NPU core 0.
pub(crate) const CORE_0: c_int = 1;

#[repr(C)]
#[derive(Default)]
struct InOutNum {
    n_input: u32,
    n_output: u32,
}

#[repr(C)]
struct Input {
    index: u32,
    buf: *mut c_void,
    size: u32,
    pass_through: u8,
    kind: c_int,
    fmt: c_int,
}

#[repr(C)]
struct Output {
    want_float: u8,
    is_prealloc: u8,
    index: u32,
    buf: *mut c_void,
    size: u32,
}

const _: () = {
    use std::mem::{offset_of, size_of};
    assert!(size_of::<Ctx>() == 8);
    assert!(size_of::<InOutNum>() == 8);
    assert!(size_of::<Input>() == 32);
    assert!(offset_of!(Input, buf) == 8);
    assert!(offset_of!(Input, size) == 16);
    assert!(offset_of!(Input, pass_through) == 20);
    assert!(offset_of!(Input, kind) == 24);
    assert!(offset_of!(Input, fmt) == 28);
    assert!(size_of::<Output>() == 24);
    assert!(offset_of!(Output, is_prealloc) == 1);
    assert!(offset_of!(Output, index) == 4);
    assert!(offset_of!(Output, buf) == 8);
    assert!(offset_of!(Output, size) == 16);
};

struct Api {
    init: unsafe extern "C" fn(*mut Ctx, *mut c_void, u32, u32, *mut c_void) -> c_int,
    destroy: unsafe extern "C" fn(Ctx) -> c_int,
    query: unsafe extern "C" fn(Ctx, c_int, *mut c_void, u32) -> c_int,
    set_core_mask: unsafe extern "C" fn(Ctx, c_int) -> c_int,
    inputs_set: unsafe extern "C" fn(Ctx, u32, *mut Input) -> c_int,
    run: unsafe extern "C" fn(Ctx, *mut c_void) -> c_int,
    outputs_get: unsafe extern "C" fn(Ctx, u32, *mut Output, *mut c_void) -> c_int,
    outputs_release: unsafe extern "C" fn(Ctx, u32, *mut Output) -> c_int,
    // The pointers above point into this library.
    _lib: Library,
}

impl Api {
    fn load(lib: &str) -> Result<Self> {
        // SAFETY: loading the vendor runtime only runs its initializers.
        let library = unsafe { Library::new(lib) }.with_context(|| format!("dlopen {lib}"))?;
        // SAFETY: each symbol is declared with its rknn_api.h signature.
        unsafe {
            Ok(Self {
                init: *library.get(b"rknn_init\0")?,
                destroy: *library.get(b"rknn_destroy\0")?,
                query: *library.get(b"rknn_query\0")?,
                set_core_mask: *library.get(b"rknn_set_core_mask\0")?,
                inputs_set: *library.get(b"rknn_inputs_set\0")?,
                run: *library.get(b"rknn_run\0")?,
                outputs_get: *library.get(b"rknn_outputs_get\0")?,
                outputs_release: *library.get(b"rknn_outputs_release\0")?,
                _lib: library,
            })
        }
    }
}

/// A loaded single-output model with float32 inputs.
pub(crate) struct Model {
    api: Api,
    ctx: Ctx,
    n_inputs: u32,
}

impl Model {
    pub(crate) fn load(lib: &str, model: &Path, core_mask: c_int) -> Result<Self> {
        let api = Api::load(lib)?;
        let mut data = std::fs::read(model).with_context(|| format!("read {}", model.display()))?;
        let size = u32::try_from(data.len()).context("model larger than 4 GiB")?;
        let mut ctx: Ctx = 0;
        // SAFETY: data outlives the call; the runtime copies what it keeps.
        let ret = unsafe {
            (api.init)(
                &mut ctx,
                data.as_mut_ptr().cast(),
                size,
                0,
                std::ptr::null_mut(),
            )
        };
        if ret < 0 {
            anyhow::bail!("rknn_init {} failed ({ret})", model.display());
        }
        let mut model = Self {
            api,
            ctx,
            n_inputs: 0,
        };
        let mut io = InOutNum::default();
        // SAFETY: valid context and an output buffer of the queried size.
        let ret = unsafe {
            (model.api.query)(
                model.ctx,
                QUERY_IN_OUT_NUM,
                (&mut io as *mut InOutNum).cast(),
                8,
            )
        };
        if ret < 0 || io.n_output != 1 {
            anyhow::bail!("unexpected rknn model io ({ret}: {} outputs)", io.n_output);
        }
        model.n_inputs = io.n_input;
        // SAFETY: valid context.
        let ret = unsafe { (model.api.set_core_mask)(model.ctx, core_mask) };
        if ret < 0 {
            log::warn!("rknn_set_core_mask({core_mask}) failed ({ret}); using the default");
        }
        Ok(model)
    }

    /// Run once on float32 `inputs` (in model order) and return the output
    /// as float32.
    pub(crate) fn run(&mut self, inputs: &mut [&mut [f32]]) -> Result<Vec<f32>> {
        if inputs.len() != self.n_inputs as usize {
            anyhow::bail!("model takes {} inputs, got {}", self.n_inputs, inputs.len());
        }
        let mut raw: Vec<Input> = inputs
            .iter_mut()
            .enumerate()
            .map(|(i, data)| -> Result<Input> {
                Ok(Input {
                    index: u32::try_from(i)?,
                    buf: data.as_mut_ptr().cast(),
                    size: u32::try_from(std::mem::size_of_val(&**data))?,
                    pass_through: 0,
                    kind: TENSOR_FLOAT32,
                    fmt: FORMAT_UNDEFINED,
                })
            })
            .collect::<Result<_>>()?;
        // SAFETY: the input buffers outlive the synchronous set + run.
        let ret = unsafe { (self.api.inputs_set)(self.ctx, self.n_inputs, raw.as_mut_ptr()) };
        if ret < 0 {
            anyhow::bail!("rknn_inputs_set failed ({ret})");
        }
        // SAFETY: valid context; no run extension.
        let ret = unsafe { (self.api.run)(self.ctx, std::ptr::null_mut()) };
        if ret < 0 {
            anyhow::bail!("rknn_run failed ({ret})");
        }
        let mut out = Output {
            want_float: 1,
            is_prealloc: 0,
            index: 0,
            buf: std::ptr::null_mut(),
            size: 0,
        };
        // SAFETY: the runtime allocates the buffer; released below.
        let ret = unsafe { (self.api.outputs_get)(self.ctx, 1, &mut out, std::ptr::null_mut()) };
        if ret < 0 || out.buf.is_null() {
            anyhow::bail!("rknn_outputs_get failed ({ret})");
        }
        // SAFETY: with want_float the runtime returns `size` bytes of f32.
        let floats = unsafe {
            std::slice::from_raw_parts(out.buf.cast::<f32>(), out.size as usize / 4).to_vec()
        };
        // SAFETY: releases the buffer obtained above.
        unsafe { (self.api.outputs_release)(self.ctx, 1, &mut out) };
        Ok(floats)
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        // SAFETY: the context is valid and no longer used.
        unsafe { (self.api.destroy)(self.ctx) };
    }
}

#[cfg(test)]
#[path = "rknn_test.rs"]
mod rknn_test;
