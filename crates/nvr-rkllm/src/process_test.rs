use std::process::Command;
use std::time::Duration;

use futures::StreamExt;

use super::*;
use crate::engine::{Backend, Run, Settings, Step};
use crate::{ChatRequest, Delta, Engine, EngineState, LlmConfig, Message, Template};

fn spawn(mode: &str) -> Engine {
    spawn_config(LlmConfig::new("fixture.rkllm", Template::Qwen), mode)
}

pub(crate) fn spawn_config(cfg: LlmConfig, mode: &str) -> Engine {
    let cancel_timeout = if matches!(mode, "run-hang" | "clear-hang") {
        Duration::from_millis(150)
    } else {
        CANCEL_TIMEOUT
    };
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "process::process_test::worker_fixture",
        "--nocapture",
    ]);
    command.env("NVR_RKLLM_WORKER_TEST", mode);
    Engine::spawn_with(Settings::from(&cfg), move |closing| {
        let mut backend = ProcessBackend::load_command(&cfg, closing, command)?;
        backend.cancel_timeout = cancel_timeout;
        Ok(backend)
    })
}

async fn ready(engine: &Engine) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), engine.wait_ready())
        .await
        .expect("worker readiness is bounded")
}

fn ask() -> ChatRequest {
    ChatRequest {
        messages: vec![Message::user("hello")],
        ..Default::default()
    }
}

async fn output(engine: &Engine) -> Vec<Delta> {
    tokio::time::timeout(
        Duration::from_secs(5),
        engine.generate(ask()).unwrap().collect(),
    )
    .await
    .expect("worker failure terminates the generation")
}

async fn failed(engine: &Engine) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(engine.state(), EngineState::Failed(_)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("worker death is observed even without a request");
    assert!(engine.generate(ask()).is_err());
}

#[tokio::test]
async fn init_abort_does_not_abort_the_host() {
    let engine = spawn("init-abort");
    assert!(ready(&engine).await.is_err());
    assert!(matches!(engine.state(), EngineState::Failed(_)));
    engine.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn run_abort_fails_active_and_queued_requests() {
    let engine = spawn("run-abort");
    ready(&engine).await.unwrap();
    let running = engine.generate(ask()).unwrap();
    let queued = engine.generate(ask()).unwrap();
    for generation in [running, queued] {
        let deltas: Vec<_> = tokio::time::timeout(Duration::from_secs(5), generation.collect())
            .await
            .unwrap();
        assert!(matches!(deltas.last(), Some(Delta::Error(_))), "{deltas:?}");
    }
    failed(&engine).await;
    engine.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn idle_worker_death_changes_health() {
    let engine = spawn("idle-abort");
    ready(&engine).await.unwrap();
    failed(&engine).await;
    engine.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn tokens_round_trip_and_runtime_errors_remain_request_errors() {
    let engine = spawn("normal");
    ready(&engine).await.unwrap();
    for _ in 0..2 {
        let deltas = output(&engine).await;
        assert_eq!(deltas[0], Delta::Content("你".into()));
        assert_eq!(deltas[1], Delta::Content("好".into()));
        assert!(matches!(deltas.last(), Some(Delta::Done { .. })));
    }
    engine.shutdown(Duration::from_secs(2)).await;

    let engine = spawn("run-error");
    ready(&engine).await.unwrap();
    assert!(matches!(
        output(&engine).await.last(),
        Some(Delta::Error(_))
    ));
    assert_eq!(engine.state(), EngineState::Ready);
    engine.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn shutdown_reaps_a_worker_stuck_in_init_or_run() {
    for mode in ["init-hang", "run-hang", "clear-hang"] {
        let engine = spawn(mode);
        let generation = if mode != "init-hang" {
            ready(&engine).await.unwrap();
            Some(engine.generate(ask()).unwrap())
        } else {
            None
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        tokio::time::timeout(
            Duration::from_secs(3),
            engine.shutdown(Duration::from_secs(2)),
        )
        .await
        .expect("shutdown cannot wait for a hung runtime");
        assert!(engine.is_stopped(), "worker has been reaped");
        drop(generation);
    }
}

#[tokio::test]
async fn cancellation_terminates_a_worker_that_never_calls_back() {
    for mode in ["run-hang", "clear-hang"] {
        let engine = spawn(mode);
        ready(&engine).await.unwrap();
        let generation = engine.generate(ask()).unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(generation);
        failed(&engine).await;
        engine.shutdown(Duration::from_secs(2)).await;
    }
}

#[tokio::test]
async fn cancellation_during_prefill_keeps_a_responsive_worker() {
    let engine = spawn("slow-prefill");
    ready(&engine).await.unwrap();
    let generation = engine.generate(ask()).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(generation);
    assert!(matches!(
        output(&engine).await.last(),
        Some(Delta::Done { .. })
    ));
    assert_eq!(engine.state(), EngineState::Ready);
    engine.shutdown(Duration::from_secs(2)).await;
}

#[tokio::test]
async fn a_worker_stuck_after_a_stop_token_is_terminated() {
    let mut cfg = LlmConfig::new("fixture.rkllm", Template::Qwen);
    cfg.stop_token_ids = vec![1];
    let engine = spawn_config(cfg, "stop-hang");
    ready(&engine).await.unwrap();
    assert!(matches!(
        output(&engine).await.last(),
        Some(Delta::Error(_))
    ));
    failed(&engine).await;
    engine.shutdown(Duration::from_secs(2)).await;
}

struct Fixture(String);

impl Backend for Fixture {
    fn run(
        &mut self,
        _: Run<'_>,
        on_token: &mut dyn FnMut(i32, &str) -> Step,
    ) -> anyhow::Result<u32> {
        match self.0.as_str() {
            "run-abort" => {
                std::thread::sleep(Duration::from_millis(150));
                std::process::abort();
            }
            "run-hang" => loop {
                std::thread::sleep(Duration::from_secs(60));
            },
            "run-error" => anyhow::bail!("fixture runtime failure"),
            "slow-prefill" => std::thread::sleep(Duration::from_millis(300)),
            _ => {}
        }
        for (id, text) in [(1, "你"), (2, "好")] {
            if on_token(id, text) == Step::Stop {
                if self.0 == "stop-hang" {
                    loop {
                        std::thread::sleep(Duration::from_secs(60));
                    }
                }
                break;
            }
        }
        Ok(7)
    }

    fn clear_history(
        &mut self,
        _: &std::sync::atomic::AtomicBool,
        _: &std::sync::atomic::AtomicBool,
    ) -> anyhow::Result<()> {
        if self.0 == "clear-hang" {
            loop {
                std::thread::sleep(Duration::from_secs(60));
            }
        }
        Ok(())
    }
}

/// Executed by a fresh test process with its stdin connected to the IPC socket.
#[test]
fn worker_fixture() {
    let Ok(mode) = std::env::var("NVR_RKLLM_WORKER_TEST") else {
        return;
    };
    if mode == "real" {
        let result = worker_with(crate::rkllm::RkllmBackend::load);
        if let Err(err) = result {
            eprintln!("real runtime worker: {err:#}");
        }
        std::process::exit(0);
    }
    let result = worker_with(|_| {
        match mode.as_str() {
            "init-abort" => std::process::abort(),
            "init-hang" => loop {
                std::thread::sleep(Duration::from_secs(60));
            },
            "idle-abort" => {
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(300));
                    std::process::abort();
                });
            }
            _ => {}
        }
        println!("native runtime stdout remains separate from IPC");
        eprintln!("native runtime stderr remains separate from IPC");
        Ok(Fixture(mode))
    });
    if let Err(err) = result {
        eprintln!("fixture: {err:#}");
    }
    std::process::exit(0);
}
