//! `nvr-asr-demo`: transcribe a complete WAV or simulate live audio input.
//!
//! Reads the file, feeds it to the [`AsrEngine`] in small chunks (optionally
//! paced in real time), and renders self-correcting interim lines plus
//! committed finals. See `README.md` for model download + native-lib setup.

use std::{path::PathBuf, time::Duration};

use anyhow::Result;
use clap::{Parser, ValueEnum};

use nvr_asr::{
    AsrConfig, AsrEngine, AsrModels, PartialPunct, SAMPLE_RATE, Transcript, load_wav_16k_mono,
    split_sentences,
};

#[derive(Clone, Copy, Debug, PartialEq, ValueEnum)]
enum Mode {
    Streaming,
    Offline,
}

#[derive(Parser)]
#[command(
    about = "Transcribe a 16 kHz WAV with Silero VAD + SenseVoice (streaming correction or offline)"
)]
struct Args {
    /// Streaming emits revisable partials; offline returns only finalized text.
    #[arg(long, value_enum, default_value = "streaming")]
    mode: Mode,
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
    /// Optional CT-Transformer punctuation model (model.onnx). When set, each
    /// final is punctuated; partials follow --partial-punct.
    #[arg(long)]
    punct: Option<PathBuf>,
    /// Partial punctuation with --punct: strip (none) or recognizer (keep
    /// SenseVoice's own marks).
    #[arg(long, default_value = "strip")]
    partial_punct: PartialPunct,
    /// Disable committing stable sentences before the speaker pauses.
    #[arg(long, default_value_t = false)]
    no_early_commit: bool,

    /// SenseVoice language: auto|zh|en|ja|ko|yue.
    #[arg(long, default_value = "auto")]
    language: String,
    /// Recognizer threads.
    #[arg(long, default_value_t = 2)]
    num_threads: i32,
    /// Recognizer provider: cpu, or rknn with an RKNN model (needs the
    /// `sherpa-shared` feature and the RKNN sherpa-onnx libraries).
    #[arg(long, default_value = "cpu")]
    provider: String,
    /// Force-split continuous speech after this many seconds (RKNN models
    /// accept a fixed input length, e.g. 10 s).
    #[arg(long, default_value_t = 10.0)]
    max_speech_secs: f32,
    /// Longest audio per recognizer decode, in seconds (the RKNN model's
    /// fixed length, e.g. 5 or 10); unlimited when omitted.
    #[arg(long)]
    max_decode_secs: Option<f32>,
    /// Committed audio re-decoded as left context after a commit, in seconds.
    #[arg(long, default_value_t = 0.5)]
    commit_overlap: f32,
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
    anyhow::ensure!(
        args.mode != Mode::Offline || !args.realtime,
        "--realtime is only supported with --mode streaming"
    );

    let samples = load_wav_16k_mono(&args.wav)?;
    log::info!(
        "loaded {} samples ({:.1}s) from {}",
        samples.len(),
        samples.len() as f32 / SAMPLE_RATE as f32,
        args.wav.display()
    );

    let mut config = AsrConfig::new(&args.model, &args.tokens, &args.vad);
    config.punct_model = args.punct;
    config.partial_punct = args.partial_punct;
    config.early_commit = !args.no_early_commit;
    config.language = args.language;
    config.num_threads = args.num_threads;
    config.provider = args.provider;
    config.max_speech_duration = args.max_speech_secs;
    config.max_decode_secs = args.max_decode_secs;
    config.commit_overlap = args.commit_overlap;
    config.partial_interval = Duration::from_millis(args.partial_ms);
    config.debug = args.debug;

    let t_load = std::time::Instant::now();
    let models = AsrModels::load(config)?;
    log::debug!(
        "engine + models load: {:.3}s",
        t_load.elapsed().as_secs_f64()
    );

    let audio_secs = samples.len() as f64 / SAMPLE_RATE as f64;
    let t_proc = std::time::Instant::now();
    if args.mode == Mode::Offline {
        let result = models.transcribe_audio(&samples)?;
        println!("{}", result.text);
        log::info!(
            "offline: {} segments, {:.2}s audio processed in {:.3}s",
            result.segments.len(),
            audio_secs,
            t_proc.elapsed().as_secs_f64()
        );
        return Ok(());
    }
    let mut engine = AsrEngine::new(models)?;
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
        Transcript::Partial { text, .. } => {
            // \r to line start, \x1b[K clears stale trailing chars; yellow marks
            // the line as interim, reset so it doesn't bleed past the text.
            print!("\r\x1b[K{YELLOW}… {text}{RESET}");
            let _ = std::io::stdout().flush();
            *pending_partial = true;
        }
        Transcript::Final {
            segment,
            text,
            start,
            duration,
        } => {
            print!("\r\x1b[K");
            for s in split_sentences(text, *start, *duration) {
                println!(
                    "#{segment:<3} [{:6.2}s +{:4.2}s] {}",
                    s.start, s.duration, s.text
                );
            }
            *pending_partial = false;
        }
    }
}
