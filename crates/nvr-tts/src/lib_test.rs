use super::*;

#[test]
fn missing_model_files_are_named() {
    let dir = std::env::temp_dir().join("nvr-tts-missing-model");
    let err = Tts::load(&TtsConfig::new(&dir, ModelKind::Vits))
        .err()
        .expect("no model");
    assert!(err.to_string().contains("model.onnx"), "{err:#}");
    let err = Tts::load(&TtsConfig::new(&dir, ModelKind::Matcha))
        .err()
        .expect("no model");
    assert!(err.to_string().contains("model-steps-3.onnx"), "{err:#}");
    let err = Tts::load(&TtsConfig::new(&dir, ModelKind::MeloRknn))
        .err()
        .expect("no model");
    assert!(err.to_string().contains("tokens.txt"), "{err:#}");
}

#[test]
fn load_first_reports_every_failure() {
    let dir = std::env::temp_dir().join("nvr-tts-missing-model");
    let err = Tts::load_first(&[
        TtsConfig::new(&dir, ModelKind::MeloRknn),
        TtsConfig::new(&dir, ModelKind::Vits),
    ])
    .err()
    .expect("nothing loads");
    let msg = err.to_string();
    assert!(msg.contains("MeloRknn") && msg.contains("Vits"), "{msg}");
}

#[test]
fn model_kinds_parse() {
    assert_eq!("vits".parse::<ModelKind>().unwrap(), ModelKind::Vits);
    assert_eq!("matcha".parse::<ModelKind>().unwrap(), ModelKind::Matcha);
    assert_eq!(
        "melo-rknn".parse::<ModelKind>().unwrap(),
        ModelKind::MeloRknn
    );
    assert!("kokoro".parse::<ModelKind>().is_err());
}

#[test]
fn audio_writes_a_16_bit_wav() {
    let audio = Audio {
        samples: vec![0.0, 0.5, -1.5],
        sample_rate: 16_000,
    };
    assert!((audio.duration_secs() - 3.0 / 16_000.0).abs() < 1e-9);
    let path = std::env::temp_dir().join(format!("nvr-tts-{}.wav", std::process::id()));
    audio.write_wav(&path).unwrap();
    let mut reader = hound::WavReader::open(&path).unwrap();
    assert_eq!(reader.spec().sample_rate, 16_000);
    let samples: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
    assert_eq!(samples, vec![0, 16384, -32767], "clamped to [-1, 1]");
    std::fs::remove_file(path).unwrap();
}

/// With a model: `NVR_TTS_TEST_MODEL_DIR=third_party/tts-models/vits-melo-tts-zh_en`.
#[test]
fn model_speaks_mixed_chinese_and_english() {
    let Ok(dir) = std::env::var("NVR_TTS_TEST_MODEL_DIR") else {
        return;
    };
    let tts = Tts::load(&TtsConfig::new(dir, ModelKind::Vits)).unwrap();
    let audio = tts
        .synthesize("你好，这是 **lite NVR** 的语音合成测试。")
        .unwrap();
    assert!(audio.duration_secs() > 1.0, "{}", audio.duration_secs());
    assert!(audio.samples.iter().any(|s| s.abs() > 0.05), "not silent");
    assert!(tts.synthesize("🎉 ***").unwrap().samples.is_empty());
}

/// On a Rockchip board: `NVR_TTS_TEST_MELO_RKNN_DIR=<MeloTTS-RKNN2 dir>`,
/// `NVR_TTS_TEST_RKNN_LIB=<librknnrt.so>` and `ORT_DYLIB_PATH`.
#[test]
fn melo_rknn_speaks_on_the_npu() {
    let Ok(dir) = std::env::var("NVR_TTS_TEST_MELO_RKNN_DIR") else {
        return;
    };
    let mut cfg = TtsConfig::new(dir, ModelKind::MeloRknn);
    if let Ok(lib) = std::env::var("NVR_TTS_TEST_RKNN_LIB") {
        cfg.rknn_lib = lib;
    }
    let tts = Tts::load(&cfg).unwrap();
    assert_eq!(tts.sample_rate(), 44_100);
    let audio = tts.synthesize("今天气温22度，适合出门。").unwrap();
    assert!(audio.duration_secs() > 1.0, "{}", audio.duration_secs());
    assert!(audio.samples.iter().any(|s| s.abs() > 0.05), "not silent");
}
