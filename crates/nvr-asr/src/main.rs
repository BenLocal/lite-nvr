//! `nvr-asr-demo`: transcribe a 16 kHz mono WAV as if it were arriving live.
//!
//! Reads the file, feeds it to the [`AsrEngine`] in small chunks (optionally
//! paced in real time), and renders self-correcting interim lines plus
//! committed finals. See `README.md` for model download + native-lib setup.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use clap::Parser;

use nvr_asr::{AsrConfig, AsrEngine, SAMPLE_RATE, Transcript, load_wav_16k_mono};

#[derive(Parser)]
#[command(
    about = "Offline-simulated-streaming ASR: transcribe a 16 kHz mono WAV as if live (Silero VAD + SenseVoice)"
)]
struct Args {
    /// SenseVoice ONNX model (e.g. model.int8.onnx).
    #[arg(long)]
    model: PathBuf,
    /// Recognizer tokens.txt.
    #[arg(long)]
    tokens: PathBuf,
    /// Silero VAD ONNX model (silero_vad.onnx).
    #[arg(long)]
    vad: PathBuf,
    /// 16 kHz mono WAV to transcribe.
    #[arg(long)]
    wav: PathBuf,
    /// Optional CT-Transformer punctuation model (model.onnx). When set,
    /// partials stay punctuation-free and each final is punctuated.
    #[arg(long)]
    punct: Option<PathBuf>,

    /// SenseVoice language: auto|zh|en|ja|ko|yue.
    #[arg(long, default_value = "auto")]
    language: String,
    /// Recognizer threads.
    #[arg(long, default_value_t = 2)]
    num_threads: i32,
    /// Simulated packet size fed per step, in milliseconds.
    #[arg(long, default_value_t = 100)]
    chunk_ms: u64,
    /// Sleep between chunks to mimic real-time arrival (off = as fast as possible).
    #[arg(long, default_value_t = false)]
    realtime: bool,
    /// Interim re-decode cadence, in milliseconds.
    #[arg(long, default_value_t = 300)]
    partial_ms: u64,
    /// Print sherpa-onnx internal debug logs.
    #[arg(long, default_value_t = false)]
    debug: bool,
}

fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();
    let args = Args::parse();

    let samples = load_wav_16k_mono(&args.wav)?;
    log::info!(
        "loaded {} samples ({:.1}s) from {}",
        samples.len(),
        samples.len() as f32 / SAMPLE_RATE as f32,
        args.wav.display()
    );

    let mut config = AsrConfig::new(&args.model, &args.tokens, &args.vad);
    config.punct_model = args.punct;
    config.language = args.language;
    config.num_threads = args.num_threads;
    config.partial_interval = Duration::from_millis(args.partial_ms);
    config.debug = args.debug;

    let t_load = std::time::Instant::now();
    let mut engine = AsrEngine::new(config)?;
    log::debug!(
        "engine + models load: {:.3}s",
        t_load.elapsed().as_secs_f64()
    );

    let audio_secs = samples.len() as f64 / SAMPLE_RATE as f64;
    let t_proc = std::time::Instant::now();
    let chunk = ((args.chunk_ms * SAMPLE_RATE as u64) / 1000).max(1) as usize;
    let mut pending_partial = false;
    for block in samples.chunks(chunk) {
        for ev in engine.accept(block) {
            render(&ev, &mut pending_partial);
        }
        if args.realtime {
            std::thread::sleep(Duration::from_millis(args.chunk_ms));
        }
    }
    for ev in engine.flush() {
        render(&ev, &mut pending_partial);
    }
    if pending_partial {
        println!();
    }
    if !args.realtime {
        let p = t_proc.elapsed().as_secs_f64();
        log::debug!(
            "processed {audio_secs:.2}s audio in {p:.3}s -> RTF {:.4} ({:.1}x real-time)",
            p / audio_secs,
            audio_secs / p
        );
    }
    Ok(())
}

/// ANSI SGR: interim partials are drawn in yellow (uncommitted, still being
/// corrected); finals use the terminal's default color (committed).
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

/// A `Partial` overwrites the current terminal line (self-correcting); a `Final`
/// commits it on its own line with timing.
fn render(ev: &Transcript, pending_partial: &mut bool) {
    use std::io::Write;
    match ev {
        Transcript::Partial { text } => {
            // \r to line start, \x1b[K clears stale trailing chars; yellow marks
            // the line as interim, reset so it doesn't bleed past the text.
            print!("\r\x1b[K{YELLOW}… {text}{RESET}");
            let _ = std::io::stdout().flush();
            *pending_partial = true;
        }
        Transcript::Final {
            text,
            start,
            duration,
        } => {
            println!("\r\x1b[K[{start:6.2}s +{duration:4.2}s] {text}");
            *pending_partial = false;
        }
    }
}
