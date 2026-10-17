use super::*;

#[test]
fn missing_runtime_is_an_error() {
    let err = Model::load("/nonexistent/librknnrt.so", Path::new("x.rknn"), CORE_0)
        .err()
        .expect("no runtime");
    assert!(err.to_string().contains("dlopen"), "{err:#}");
}
