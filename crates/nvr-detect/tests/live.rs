//! End-to-end smoke against a real ONNX model. Ignored by default; needs a
//! weights file. Run:
//!   DETECT_TEST_MODEL=third_party/detect-models/yolov8n.onnx \
//!   DETECT_TEST_IMAGE=crates/nvr-detect/tests/bus.jpg \
//!     cargo test -p nvr-detect --test live -- --ignored --nocapture

use nvr_detect::{Detector, DetectorConfig, UslsDetector};

#[test]
#[ignore]
fn detects_on_a_real_image() {
    let model = std::env::var("DETECT_TEST_MODEL").expect("set DETECT_TEST_MODEL");
    let image = std::env::var("DETECT_TEST_IMAGE").expect("set DETECT_TEST_IMAGE");

    let cfg = DetectorConfig {
        name: "yolo".into(),
        model_file: model.clone(),
        version: None,
        scale: None,
        input_size: 640,
        conf: 0.25,
        iou: 0.45,
        class_names: vec![],
        device: "cpu".into(),
    };
    let det = UslsDetector::new(&cfg, std::path::Path::new(&model)).expect("build detector");

    let img = image::open(&image).expect("open image").to_rgb8();
    let (w, h) = img.dimensions();
    let dets = det.detect(img.as_raw(), w, h).expect("detect");

    println!("found {} detections", dets.len());
    for d in &dets {
        println!("  {} {:.2} {:?}", d.label, d.confidence, d.bbox);
    }
    assert!(!dets.is_empty(), "expected at least one detection");
}
