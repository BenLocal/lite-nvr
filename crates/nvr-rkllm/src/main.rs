//! `nvr-rkllm-demo`: load a `.rkllm` model and chat with it on the terminal,
//! one question per line, keeping the conversation history.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use futures::StreamExt;

use nvr_rkllm::{ChatRequest, Delta, Engine, LlmConfig, Message, Template};

#[derive(Parser)]
#[command(about = "Chat with an RKLLM model on a Rockchip board")]
struct Args {
    /// `.rkllm` model.
    #[arg(long)]
    model: PathBuf,
    /// qwen or deepseek-r1.
    #[arg(long, default_value = "qwen")]
    template: Template,
    /// librkllmrt.so path (default: searched on LD_LIBRARY_PATH).
    #[arg(long, default_value = "librkllmrt.so")]
    lib: String,
    #[arg(long)]
    system: Option<String>,
    #[arg(long, default_value_t = 512)]
    max_tokens: u32,
    /// Let the model think first (printed dimmed).
    #[arg(long)]
    think: bool,
}

fn main() -> Result<()> {
    nvr_rkllm::run_worker_if_requested();
    run()
}

#[tokio::main(flavor = "current_thread")]
async fn run() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();
    let mut cfg = LlmConfig::new(&args.model, args.template);
    cfg.lib = args.lib;
    let engine = Engine::spawn(cfg);
    let started = std::time::Instant::now();
    engine.wait_ready().await?;
    eprintln!(
        "loaded in {:.1?}; type a question, empty line to quit",
        started.elapsed()
    );

    let mut history: Vec<Message> = args.system.into_iter().map(Message::system).collect();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            break;
        }
        history.push(Message::user(line));
        let req = ChatRequest {
            messages: history.clone(),
            max_tokens: Some(args.max_tokens),
            enable_thinking: args.think,
            ..Default::default()
        };
        let asked = std::time::Instant::now();
        let mut first = None;
        let mut answer = String::new();
        let mut generation = engine.generate(req)?;
        while let Some(delta) = generation.next().await {
            first.get_or_insert_with(|| asked.elapsed());
            match delta {
                Delta::Reasoning(t) => print!("\x1b[2m{t}\x1b[0m"),
                Delta::Content(t) => {
                    print!("{t}");
                    answer.push_str(&t);
                }
                Delta::Done {
                    usage,
                    finish_reason,
                } => {
                    let total = asked.elapsed().as_secs_f32();
                    eprintln!(
                        "\n[{finish_reason:?}] prompt {} + completion {} tokens, first output {:.1?}, {:.1} tok/s",
                        usage.prompt_tokens,
                        usage.completion_tokens,
                        first.unwrap_or_default(),
                        usage.completion_tokens as f32 / total
                    );
                }
                Delta::Error(e) => eprintln!("\nerror: {e}"),
            }
            std::io::stdout().flush()?;
        }
        history.push(Message::assistant(answer));
    }
    Ok(())
}
