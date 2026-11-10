use super::{Commit, partial_text, stable_prefix, strip_punct};
use crate::PartialPunct;

#[test]
fn strip_punct_removes_cjk_and_ascii() {
    // Chinese sentence punctuation.
    assert_eq!(
        strip_punct("派饭时间，早上9点至下午5点。"),
        "派饭时间早上9点至下午5点"
    );
    assert_eq!(strip_punct("你好、世界；测试：完成！"), "你好世界测试完成");
    // ASCII punctuation (spaces are kept).
    assert_eq!(strip_punct("hello, world!"), "hello world");
}

#[test]
fn strip_punct_keeps_digits_and_letters() {
    assert_eq!(strip_punct("9点abc九"), "9点abc九");
}

// Requires real models on disk; run manually with:
//   SHERPA_ONNX_LIB_DIR=... cargo test -p nvr-asr -- --ignored shared_models
#[test]
#[ignore = "needs ~1.3GB models under third_party/asr-models"]
fn shared_models_drive_two_independent_engines() {
    use crate::{AsrConfig, AsrEngine, AsrModels};
    let base = std::path::Path::new("third_party/asr-models");
    let sv = base.join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17");
    let cfg = AsrConfig::new(
        sv.join("model.int8.onnx"),
        sv.join("tokens.txt"),
        base.join("silero_vad.onnx"),
    );
    let models = AsrModels::load(cfg).expect("load models");
    let mut a = AsrEngine::new(models.clone()).expect("engine a");
    let mut b = AsrEngine::new(models.clone()).expect("engine b");
    let silence = vec![0.0f32; 16_000];
    let _ = a.accept(&silence);
    let _ = b.accept(&silence);
    assert!(std::sync::Arc::strong_count(&models) >= 3); // models + a + b
}

#[test]
#[ignore = "requires SenseVoice, Silero VAD and a speech WAV; see docs/asr.md"]
fn complete_audio_matches_streaming_and_skips_partials() {
    use std::{path::PathBuf, time::Instant};

    use crate::{
        AsrConfig, AsrEngine, AsrModels, Transcript, TranscriptionSegment, load_wav_16k_mono,
    };

    let base = std::env::var_os("NVR_ASR_TEST_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/asr-models")
        });
    let sense_voice = base.join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17");
    let wav = std::env::var_os("NVR_ASR_TEST_WAV")
        .map(PathBuf::from)
        .unwrap_or_else(|| sense_voice.join("test_wavs/zh.wav"));
    let models = AsrModels::load(AsrConfig::new(
        sense_voice.join("model.int8.onnx"),
        sense_voice.join("tokens.txt"),
        base.join("silero_vad.onnx"),
    ))
    .expect("load real models");
    let samples = load_wav_16k_mono(&wav).expect("load speech WAV");

    // Warm native inference before comparing processing time (no wall-clock pacing).
    models.transcribe_audio(&samples).expect("warm up");
    let mut streaming = AsrEngine::new(models.clone()).expect("streaming engine");
    let started = Instant::now();
    let mut events = Vec::new();
    for block in samples.chunks(1600) {
        events.extend(streaming.accept(block));
    }
    events.extend(streaming.flush());
    let streaming_elapsed = started.elapsed();
    let partials = events.iter().filter(|event| !event.is_final()).count();
    assert!(
        partials > 0,
        "fixture must exercise live partial recognition"
    );
    let expected: Vec<_> = events
        .into_iter()
        .filter_map(|event| match event {
            Transcript::Final {
                text,
                start,
                duration,
                ..
            } => Some(TranscriptionSegment {
                text,
                start,
                duration,
            }),
            Transcript::Partial { .. } => None,
        })
        .collect();
    assert!(
        !expected.is_empty(),
        "fixture must contain recognizable speech"
    );

    let started = Instant::now();
    let complete = models.transcribe_audio(&samples).expect("complete audio");
    let offline_elapsed = started.elapsed();
    assert_eq!(complete.segments, expected);
    assert_eq!(
        complete.text,
        complete
            .segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(
        models.transcribe_file(&wav).expect("complete WAV"),
        complete
    );

    // The offline path must skip interim decoding, rather than merely filtering its output.
    let mut offline = AsrEngine::new(models.clone()).expect("offline engine");
    let mut speech_windows = 0;
    for block in samples.chunks(1600) {
        assert!(
            offline
                .accept_samples(block, false)
                .iter()
                .all(Transcript::is_final)
        );
        if offline.vad.detected() {
            speech_windows += 1;
        }
        assert!(offline.speech_buf.is_empty());
        assert_eq!(offline.since_partial, 0);
    }
    assert!(speech_windows > 0);
    assert!(offline.flush().iter().all(Transcript::is_final));
    let empty = models.transcribe_audio(&[]).expect("empty audio");
    assert!(empty.text.is_empty() && empty.segments.is_empty());
    let silence = models
        .transcribe_audio(&vec![0.0; 16_000])
        .expect("silence");
    assert!(silence.text.is_empty() && silence.segments.is_empty());
    eprintln!(
        "audio={:.3}s partials={partials} segments={} streaming={:.3}s offline={:.3}s text={:?}",
        samples.len() as f64 / 16_000.0,
        complete.segments.len(),
        streaming_elapsed.as_secs_f64(),
        offline_elapsed.as_secs_f64(),
        complete.text
    );
}

#[test]
fn partial_text_strips_only_with_punct_model_and_strip_mode() {
    let raw = || "你好，世界。".to_string();
    assert_eq!(partial_text(raw(), true, PartialPunct::Strip), "你好世界");
    assert_eq!(
        partial_text(raw(), true, PartialPunct::Recognizer),
        "你好，世界。"
    );
    assert_eq!(
        partial_text(raw(), false, PartialPunct::Strip),
        "你好，世界。"
    );
}

fn toks(s: &[&str]) -> Vec<String> {
    s.iter().map(|t| t.to_string()).collect()
}

#[test]
fn stable_prefix_ignores_the_trailing_mark() {
    // SenseVoice ends every decode with a mark; it is not a pause.
    let t = toks(&["开", "放", "时", "间", "。"]);
    assert_eq!(stable_prefix(&t, &[0.1, 0.2, 0.3, 0.4, 0.5]), None);
}

#[test]
fn stable_prefix_cuts_before_the_next_sentence() {
    let t = toks(&["你", "好", "。", "再", "见", "。"]);
    let c = stable_prefix(&t, &[0.2, 0.4, 0.9, 1.5, 1.7, 2.0]).unwrap();
    assert_eq!(c.text, "你好。");
    assert!((c.cut_secs - 1.38).abs() < 1e-4);
}

#[test]
fn stable_prefix_takes_the_last_inner_boundary_and_keeps_cut_after_mark() {
    let t = toks(&["好", "？", "！", "嗯", "。", "走", "。"]);
    let c = stable_prefix(&t, &[0.1, 0.3, 0.35, 0.5, 0.6, 0.65, 0.9]).unwrap();
    assert_eq!(
        c,
        Commit {
            text: "好？！嗯。".into(),
            // next token at 0.65 - margin would precede the mark at 0.6.
            cut_secs: 0.6,
        }
    );
}

#[test]
fn stable_prefix_joins_sentencepiece_words_and_needs_timestamps() {
    let t = toks(&["\u{2581}hello", "\u{2581}world", ".", "\u{2581}bye", "."]);
    let c = stable_prefix(&t, &[0.0, 0.3, 0.6, 1.0, 1.2]).unwrap();
    assert_eq!(c.text, "hello world.");
    assert_eq!(stable_prefix(&t, &[]), None);
}

#[test]
#[ignore = "requires SenseVoice, Silero VAD and a speech WAV; see docs/asr.md"]
fn early_commit_splits_two_sentences_inside_one_vad_segment() {
    use std::path::PathBuf;

    use crate::{AsrConfig, AsrEngine, AsrModels, Transcript, load_wav_16k_mono};

    let base = std::env::var_os("NVR_ASR_TEST_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../third_party/asr-models")
        });
    let sense_voice = base.join("sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17");
    let wav = std::env::var_os("NVR_ASR_TEST_WAV")
        .map(PathBuf::from)
        .unwrap_or_else(|| sense_voice.join("test_wavs/zh.wav"));
    let mut cfg = AsrConfig::new(
        sense_voice.join("model.int8.onnx"),
        sense_voice.join("tokens.txt"),
        base.join("silero_vad.onnx"),
    );
    cfg.max_speech_duration = 30.0;
    let models = AsrModels::load(cfg).expect("load real models");
    let once = load_wav_16k_mono(&wav).expect("load speech WAV");
    // Continuous speech: drop the clip's leading/trailing silence so the copies
    // join without a pause the VAD would split on.
    let speech = &once[(0.65 * 16_000.0) as usize..(5.25 * 16_000.0) as usize];
    let twice = [speech, speech].concat();

    let mut engine = AsrEngine::new(models).expect("engine");
    let mut events = Vec::new();
    for block in twice.chunks(1600) {
        events.extend(engine.accept(block));
    }
    let before_flush = events.iter().filter(|e| e.is_final()).count();
    events.extend(engine.flush());
    for e in &events {
        eprintln!("{e:?}");
    }

    let finals: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Transcript::Final {
                segment,
                text,
                start,
                duration,
            } => Some((*segment, text.clone(), *start, *duration)),
            Transcript::Partial { .. } => None,
        })
        .collect();
    assert!(
        before_flush >= 1,
        "first sentence must commit while speech continues"
    );
    assert_eq!(finals.len(), 2, "one final per sentence: {finals:?}");
    assert_eq!(finals[0].0 + 1, finals[1].0, "each final opens a new id");
    for (_, text, ..) in &finals {
        // Each copy is one sentence: the cut neither repeats nor drops words.
        assert!(text.contains("时间早上9点至下午5点"), "{finals:?}");
        assert_eq!(text.matches("早上").count(), 1, "{finals:?}");
    }
    let first_end = finals[0].2 + finals[0].3;
    assert!(
        (finals[1].2 - first_end).abs() < 1e-3,
        "finals tile the timeline"
    );
}

#[test]
fn stable_prefix_commits_at_clause_comma_but_not_list_separator() {
    // SenseVoice joins continuous sentences with `，`.
    let t = toks(&["点", "，", "开", "放", "。"]);
    let c = stable_prefix(&t, &[0.1, 0.4, 0.5, 0.7, 0.9]).unwrap();
    assert_eq!(c.text, "点，");
    let t = toks(&["甲", "、", "乙", "。"]);
    assert_eq!(stable_prefix(&t, &[0.1, 0.2, 0.3, 0.4]), None);
}
