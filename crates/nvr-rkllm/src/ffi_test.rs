use super::*;

#[test]
fn missing_library_is_an_error() {
    let err = load("/nonexistent/librkllmrt.so")
        .err()
        .expect("no library");
    assert!(err.to_string().contains("dlopen"), "{err:#}");
}

/// Spot-check the generated layout against g++'s view of the same header, on
/// top of bindgen's own per-struct layout tests.
#[test]
fn generated_layout_matches_gxx() {
    use std::mem::size_of;
    assert_eq!(size_of::<RKLLMParam>(), 184);
    assert_eq!(size_of::<RKLLMInput>(), 160);
    assert_eq!(size_of::<RKLLMInferParam>(), 40);
    assert_eq!(size_of::<RKLLMResult>(), 72);
    assert_eq!(size_of::<RKLLMCallback>(), 48);
    assert_eq!(BIG_CORES_MASK, 0xf0);
}
