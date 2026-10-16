//! Generates dlopen bindings for the vendored RKLLM runtime header
//! (`include/rkllm.h`, runtime 1.3.0). The header is parsed as freestanding C
//! so neither host nor cross builds need a target sysroot.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=include");
    let bindings = bindgen::Builder::default()
        .header("include/rkllm.h")
        .clang_args(["-x", "c", "-ffreestanding", "-Iinclude/shim"])
        .allowlist_function("rkllm_.*")
        .allowlist_type("RKLLM.*|LLM.*")
        .allowlist_var("CPU[0-7]")
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false)
        .derive_default(true)
        .dynamic_library_name("RkllmLib")
        .dynamic_link_require_all(true)
        .wrap_unsafe_ops(true)
        .generate()
        .expect("generate rkllm.h bindings");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("rkllm.rs");
    bindings.write_to_file(out).expect("write rkllm.h bindings");
}
