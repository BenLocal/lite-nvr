use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;

use super::*;
use crate::{Message, Template};

/// Emits a fixed token script, optionally waiting for a release before each
/// token, and records how many tokens the engine accepted.
struct Script {
    tokens: Vec<(i32, &'static str)>,
    gate: Option<Mutex<Receiver<()>>>,
    accepted: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl Backend for Script {
    fn run(&mut self, run: Run<'_>, on_token: &mut dyn FnMut(i32, &str) -> Step) -> Result<u32> {
        self.prompts.lock().unwrap().push(run.prompt.to_string());
        for (n, (id, text)) in self.tokens.iter().enumerate() {
            if n as i32 >= run.max_new_tokens {
                break;
            }
            if let Some(gate) = &self.gate {
                gate.lock().unwrap().recv().ok();
            }
            if on_token(*id, text) == Step::Stop {
                break;
            }
            self.accepted.fetch_add(1, Ordering::SeqCst);
        }
        Ok(7)
    }
}

fn settings(template: Template, queue_depth: usize) -> Settings {
    Settings {
        template,
        max_new_tokens: 64,
        stop_token_ids: vec![99],
        queue_depth,
    }
}

fn script(tokens: Vec<(i32, &'static str)>) -> (Script, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
    let accepted = Arc::new(AtomicUsize::new(0));
    let prompts = Arc::new(Mutex::new(Vec::new()));
    let s = Script {
        tokens,
        gate: None,
        accepted: Arc::clone(&accepted),
        prompts: Arc::clone(&prompts),
    };
    (s, accepted, prompts)
}

fn ask(text: &str) -> ChatRequest {
    ChatRequest {
        messages: vec![Message::user(text)],
        ..Default::default()
    }
}

async fn collect(g: Generation) -> Vec<Delta> {
    tokio::time::timeout(Duration::from_secs(5), g.collect())
        .await
        .expect("generation finished")
}

#[tokio::test]
async fn streams_content_and_stops_on_a_stop_token() {
    let (s, _, prompts) = script(vec![(1, "你"), (2, "好"), (99, ""), (3, "不该出现")]);
    let engine = Engine::spawn_with(settings(Template::Qwen, 2), move || Ok(s));
    engine.wait_ready().await.unwrap();
    let deltas = collect(engine.generate(ask("hi")).unwrap()).await;
    assert_eq!(
        deltas,
        vec![
            Delta::Content("你".into()),
            Delta::Content("好".into()),
            Delta::Done {
                usage: Usage {
                    prompt_tokens: 7,
                    completion_tokens: 2
                },
                finish_reason: FinishReason::Stop
            }
        ]
    );
    assert!(prompts.lock().unwrap()[0].contains("<|im_start|>user\nhi<|im_end|>"));
}

#[tokio::test]
async fn hitting_max_tokens_reports_length() {
    let (s, _, _) = script(vec![(1, "a"), (2, "b"), (3, "c")]);
    let engine = Engine::spawn_with(settings(Template::Qwen, 2), move || Ok(s));
    engine.wait_ready().await.unwrap();
    let mut req = ask("hi");
    req.max_tokens = Some(2);
    let deltas = collect(engine.generate(req).unwrap()).await;
    assert!(matches!(
        deltas.last(),
        Some(Delta::Done {
            finish_reason: FinishReason::Length,
            usage: Usage {
                completion_tokens: 2,
                ..
            }
        })
    ));
}

#[tokio::test]
async fn reasoning_is_split_from_the_answer() {
    let (s, _, _) = script(vec![(1, "想"), (2, "</think>"), (3, "\n\n答")]);
    let engine = Engine::spawn_with(settings(Template::DeepseekR1, 2), move || Ok(s));
    engine.wait_ready().await.unwrap();
    let mut req = ask("hi");
    req.enable_thinking = true;
    let deltas = collect(engine.generate(req).unwrap()).await;
    assert_eq!(deltas[0], Delta::Reasoning("想".into()));
    assert_eq!(deltas[1], Delta::Content("答".into()));
}

#[tokio::test]
async fn failed_load_is_reported_and_never_retried() {
    let engine =
        Engine::spawn_with::<Script, _>(settings(Template::Qwen, 2), || anyhow::bail!("bad model"));
    let err = engine.wait_ready().await.unwrap_err();
    assert!(err.to_string().contains("bad model"));
    assert_eq!(engine.state(), EngineState::Failed("bad model".into()));
    assert!(matches!(
        engine.generate(ask("hi")),
        Err(GenerateError::NotReady(EngineState::Failed(_)))
    ));
}

#[tokio::test]
async fn requests_beyond_the_queue_are_busy() {
    let (mut s, _, _) = script(vec![(1, "a")]);
    let (release, gate) = channel();
    s.gate = Some(Mutex::new(gate));
    let engine = Engine::spawn_with(settings(Template::Qwen, 1), move || Ok(s));
    engine.wait_ready().await.unwrap();
    let running = engine.generate(ask("1")).unwrap();
    // Wait until the first job left the queue and is blocked in the backend.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let queued = engine.generate(ask("2")).unwrap();
    assert!(matches!(
        engine.generate(ask("3")),
        Err(GenerateError::Busy)
    ));
    release.send(()).unwrap();
    release.send(()).unwrap();
    assert!(matches!(
        collect(running).await.last(),
        Some(Delta::Done { .. })
    ));
    assert!(matches!(
        collect(queued).await.last(),
        Some(Delta::Done { .. })
    ));
}

#[tokio::test]
async fn dropping_the_generation_stops_the_backend() {
    let (mut s, accepted, _) = script(vec![(1, "a"), (2, "b"), (3, "c"), (4, "d")]);
    let (release, gate) = channel();
    s.gate = Some(Mutex::new(gate));
    let engine = Engine::spawn_with(settings(Template::Qwen, 2), move || Ok(s));
    engine.wait_ready().await.unwrap();
    let mut g = engine.generate(ask("hi")).unwrap();
    release.send(()).unwrap();
    assert_eq!(g.next().await, Some(Delta::Content("a".into())));
    drop(g);
    for _ in 0..3 {
        let _ = release.send(());
    }
    // A later request still runs, proving the first one ended.
    let next = engine.generate(ask("again")).unwrap();
    for _ in 0..4 {
        let _ = release.send(());
    }
    collect(next).await;
    // One token from the cancelled run, four from the second.
    assert_eq!(accepted.load(Ordering::SeqCst), 5);
}
