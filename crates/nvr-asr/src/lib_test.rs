use std::time::Duration;

use super::{AsrConfig, PartialPunct, Transcript, split_sentences};

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
    let p = Transcript::Partial {
        segment: 3,
        text: "hi".into(),
    };
    let f = Transcript::Final {
        segment: 4,
        text: "hello".into(),
        start: 1.0,
        duration: 0.5,
    };
    assert_eq!(p.text(), "hi");
    assert_eq!(p.segment(), 3);
    assert_eq!(f.segment(), 4);
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
    assert_eq!(c.partial_punct, PartialPunct::Strip);
    assert_eq!(c.provider, "cpu");
}

#[test]
fn partial_punct_parses_modes() {
    assert_eq!(
        "strip".parse::<PartialPunct>().unwrap(),
        PartialPunct::Strip
    );
    assert_eq!("none".parse::<PartialPunct>().unwrap(), PartialPunct::Strip);
    assert_eq!(
        " Recognizer ".parse::<PartialPunct>().unwrap(),
        PartialPunct::Recognizer
    );
    assert!("model".parse::<PartialPunct>().is_err());
}

#[test]
fn split_sentences_on_sentence_ends_with_proportional_time() {
    let s = split_sentences("今天开会。讨论实时转写！好吗？", 10.0, 6.0);
    let texts: Vec<_> = s.iter().map(|x| x.text.as_str()).collect();
    assert_eq!(texts, ["今天开会。", "讨论实时转写！", "好吗？"]);
    // 5 + 7 + 3 = 15 characters over 6 seconds.
    assert!((s[0].start - 10.0).abs() < 1e-4);
    assert!((s[0].duration - 2.0).abs() < 1e-4);
    assert!((s[1].start - 12.0).abs() < 1e-4);
    assert!((s[2].start + s[2].duration - 16.0).abs() < 1e-4);
}

#[test]
fn split_sentences_keeps_decimals_runs_and_tail() {
    let texts = |t: &str| {
        split_sentences(t, 0.0, 1.0)
            .into_iter()
            .map(|x| x.text)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        texts("温度3.5度。真的吗？！还没说完"),
        ["温度3.5度。", "真的吗？！", "还没说完"]
    );
    assert_eq!(texts("Hello world. Bye."), ["Hello world.", "Bye."]);
    assert_eq!(texts("没有句号"), ["没有句号"]);
    assert!(texts("").is_empty());
}

#[test]
fn preroll_covers_vad_detection_delay() {
    let mut c = AsrConfig::new("m", "t", "v");
    c.min_speech_duration = 0.25;
    // (0.25 + 0.3) s at 16 kHz.
    assert_eq!(c.preroll_samples(), 8800);
}
