fn main() {
    // See src/libstdcxx_compat.cpp. Whole-archive keeps the shim even though
    // the sherpa-onnx libs that need it come later on the link line.
    println!("cargo:rerun-if-changed=src/libstdcxx_compat.cpp");
    cc::Build::new()
        .cpp(true)
        .file("src/libstdcxx_compat.cpp")
        .link_lib_modifier("+whole-archive")
        .compile("nvr_asr_libstdcxx_compat");
}
