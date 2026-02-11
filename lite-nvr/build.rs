fn main() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("linux") {
        println!("cargo:rustc-cfg=feature=\"linux\"");
    }
}
