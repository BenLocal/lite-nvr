//! `nvr-tts-demo`: synthesize text sentence by sentence, as the voice chat
//! will, and report per-sentence latency and the real-time factor.

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::Parser;

use nvr_tts::{Audio, ModelKind, SentenceSplitter, Tts, TtsConfig};

#[derive(Parser)]
#[command(about = "Synthesize text with a sherpa-onnx TTS model and report RTF")]
struct Args {
    /// Model directory (e.g. third_party/tts-models/vits-melo-tts-zh_en).
    #[arg(long)]
    model_dir: PathBuf,
    /// vits, matcha or melo-rknn.
    #[arg(long, default_value = "vits")]
    kind: ModelKind,
    #[arg(long)]
    text: String,
    /// Output WAV (all sentences joined).
    #[arg(long, default_value = "tts.wav")]
    out: PathBuf,
    #[arg(long, default_value_t = 4)]
    threads: i32,
    #[arg(long, default_value_t = 1.0)]
    speed: f32,
    #[arg(long, default_value_t = 0)]
    sid: i32,
    /// VITS weights file in the model directory (default: model.onnx).
    #[arg(long)]
    model_file: Option<String>,
    /// librknnrt.so for melo-rknn (default: searched on LD_LIBRARY_PATH).
    #[arg(long, default_value = "librknnrt.so")]
    rknn_lib: String,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();
    let mut cfg = TtsConfig::new(&args.model_dir, args.kind);
    cfg.num_threads = args.threads;
    cfg.speed = args.speed;
    cfg.speaker_id = args.sid;
    cfg.model_file = args.model_file;
    cfg.rknn_lib = args.rknn_lib;

    let started = Instant::now();
    let tts = Tts::load(&cfg)?;
    println!(
        "loaded in {:.2?}, {} Hz",
        started.elapsed(),
        tts.sample_rate()
    );

    let mut splitter = SentenceSplitter::new();
    let mut sentences = splitter.push(&args.text);
    sentences.extend(splitter.finish());

    let mut all = Audio {
        samples: Vec::new(),
        sample_rate: tts.sample_rate(),
    };
    let (mut busy, mut spoken) = (0.0f32, 0.0f32);
    for sentence in &sentences {
        let t = Instant::now();
        let audio = tts.synthesize(sentence)?;
        let took = t.elapsed().as_secs_f32();
        busy += took;
        spoken += audio.duration_secs();
        println!(
            "{took:6.2}s for {:5.2}s audio (RTF {:.2})  {sentence}",
            audio.duration_secs(),
            took / audio.duration_secs().max(f32::EPSILON)
        );
        all.samples.extend_from_slice(&audio.samples);
    }
    println!(
        "total: {busy:.2}s for {spoken:.2}s audio, RTF {:.2}",
        busy / spoken.max(f32::EPSILON)
    );
    all.write_wav(&args.out)?;
    println!("wrote {}", args.out.display());
    Ok(())
}
