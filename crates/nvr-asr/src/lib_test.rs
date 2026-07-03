use std::time::Duration;

use super::{AsrConfig, Transcript};

#[test]
fn partial_interval_samples_from_duration() {
    let mut c = AsrConfig::new("m", "t", "v");
    c.partial_interval = Duration::from_millis(300);
    c.vad_window_size = 512;
    // 0.3s * 16000 Hz = 4800 samples.
    assert_eq!(c.partial_interval_samples(), 4800);
}

#[test]
fn partial_interval_at_least_one_window() {
    let mut c = AsrConfig::new("m", "t", "v");
    c.partial_interval = Duration::from_millis(0);
    c.vad_window_size = 512;
    assert_eq!(c.partial_interval_samples(), 512);
}

#[test]
fn transcript_accessors() {
    let p = Transcript::Partial { text: "hi".into() };
    let f = Transcript::Final {
        text: "hello".into(),
        start: 1.0,
        duration: 0.5,
    };
    assert_eq!(p.text(), "hi");
    assert!(!p.is_final());
    assert_eq!(f.text(), "hello");
    assert!(f.is_final());
}

#[test]
fn config_new_sets_paths_and_defaults() {
    let c = AsrConfig::new("model.onnx", "tokens.txt", "vad.onnx");
    assert_eq!(c.sense_voice_model.to_str().unwrap(), "model.onnx");
    assert_eq!(c.tokens.to_str().unwrap(), "tokens.txt");
    assert_eq!(c.silero_vad_model.to_str().unwrap(), "vad.onnx");
    assert_eq!(c.language, "auto");
    assert!(c.use_itn);
    assert_eq!(c.vad_window_size, 512);
}
