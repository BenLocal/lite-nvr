//! RKLLM runs after exec in a disposable child. Stdin is a private duplex
//! socket; stdout and stderr remain available for the vendor's native logs.

use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::engine::{Backend, Run, Step};
use crate::{LlmConfig, Sampling};

const WORKER_ARG: &str = "--nvr-rkllm-worker";
const POLL: Duration = Duration::from_millis(50);
const LOAD_TIMEOUT: Duration = Duration::from_secs(120);
// Prefill can take seconds before the first callback. Give cancellation a
// grace period; shutdown, in contrast, terminates the worker immediately.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(30);
const ABORT_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
enum Request {
    Init {
        cfg: LlmConfig,
        parent: u32,
    },
    Clear,
    Run {
        prompt: String,
        max_new_tokens: i32,
        sampling: Sampling,
    },
    Step(bool),
}

#[derive(Serialize, Deserialize)]
enum Response {
    Ready,
    Cleared,
    Token { id: i32, text: String },
    Done(u32),
    Error(String),
}

struct Channel(UnixStream);

impl Channel {
    fn new(socket: UnixStream) -> Result<Self> {
        socket.set_read_timeout(Some(POLL))?;
        socket.set_write_timeout(Some(POLL))?;
        Ok(Self(socket))
    }

    fn send(
        &mut self,
        value: &impl Serialize,
        check: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        let data = serde_json::to_vec(value)?;
        if data.len() > MAX_FRAME {
            bail!("worker frame exceeds {MAX_FRAME} bytes");
        }
        let length = u32::try_from(data.len())?.to_be_bytes();
        for mut remaining in [length.as_slice(), data.as_slice()] {
            while !remaining.is_empty() {
                check()?;
                match self.0.write(remaining) {
                    Ok(0) => bail!("worker socket closed while writing"),
                    Ok(n) => remaining = &remaining[n..],
                    Err(err) if retry(&err) => continue,
                    Err(err) => return Err(err).context("write worker socket"),
                }
            }
        }
        Ok(())
    }

    fn read_exact(
        &mut self,
        mut remaining: &mut [u8],
        check: &mut dyn FnMut() -> Result<()>,
    ) -> Result<()> {
        // Keep partial frames across timeout/interrupt retries.
        while !remaining.is_empty() {
            check()?;
            match self.0.read(remaining) {
                Ok(0) => bail!("worker socket closed while reading"),
                Ok(n) => remaining = &mut remaining[n..],
                Err(err) if retry(&err) => continue,
                Err(err) => return Err(err).context("read worker socket"),
            }
        }
        Ok(())
    }

    fn recv<T: DeserializeOwned>(&mut self, check: &mut dyn FnMut() -> Result<()>) -> Result<T> {
        let mut length = [0; 4];
        self.read_exact(&mut length, check)?;
        let length = usize::try_from(u32::from_be_bytes(length))?;
        if length > MAX_FRAME {
            bail!("worker frame exceeds {MAX_FRAME} bytes");
        }
        let mut data = vec![0; length];
        self.read_exact(&mut data, check)?;
        serde_json::from_slice(&data).context("decode worker frame")
    }
}

fn retry(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::Interrupted
    )
}

pub(crate) struct ProcessBackend {
    child: Child,
    channel: Channel,
    failed: Option<String>,
    cancel_timeout: Duration,
}

impl ProcessBackend {
    pub(crate) fn load(cfg: &LlmConfig, closing: &AtomicBool) -> Result<Self> {
        crate::rkllm::check_model(&cfg.model)?;
        let mut command =
            Command::new(std::env::current_exe().context("locate RKLLM worker executable")?);
        command.arg(WORKER_ARG);
        Self::load_command(cfg, closing, command)
    }

    pub(crate) fn load_command(
        cfg: &LlmConfig,
        closing: &AtomicBool,
        mut command: Command,
    ) -> Result<Self> {
        let (socket, worker_socket) = UnixStream::pair().context("create RKLLM worker socket")?;
        let channel = Channel::new(socket)?;
        let child = command
            .stdin(Stdio::from(OwnedFd::from(worker_socket)))
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .context("spawn RKLLM worker")?;
        // Command retains its configured Stdio after spawn. Close the host's
        // copy now, or an init-time crash won't produce EOF on our socket.
        drop(command);
        let mut backend = Self {
            child,
            channel,
            failed: None,
            cancel_timeout: CANCEL_TIMEOUT,
        };
        let started = Instant::now();
        let mut check = || {
            if closing.load(Ordering::Relaxed) {
                bail!("shutting down while loading model");
            }
            if started.elapsed() >= LOAD_TIMEOUT {
                bail!("RKLLM worker model load timed out");
            }
            Ok(())
        };
        backend.send(
            &Request::Init {
                cfg: cfg.clone(),
                parent: std::process::id(),
            },
            &mut check,
        )?;
        match backend.recv(&mut check)? {
            Response::Ready => Ok(backend),
            Response::Error(err) => bail!("worker model load failed: {err}"),
            _ => bail!("unexpected worker load response"),
        }
    }

    fn send(&mut self, request: &Request, check: &mut dyn FnMut() -> Result<()>) -> Result<()> {
        let result = self.channel.send(request, check);
        self.transport(result)
    }

    fn recv(&mut self, check: &mut dyn FnMut() -> Result<()>) -> Result<Response> {
        let result = self.channel.recv(check);
        self.transport(result)
    }

    fn transport<T>(&mut self, result: Result<T>) -> Result<T> {
        result.map_err(|err| {
            let status = self.child.try_wait().ok().flatten();
            let err = err.context(format!("RKLLM worker communication failed ({status:?})"));
            self.failed = Some(format!("{err:#}"));
            err
        })
    }
}

impl Backend for ProcessBackend {
    fn run(&mut self, run: Run<'_>, on_token: &mut dyn FnMut(i32, &str) -> Step) -> Result<u32> {
        let mut stopping = None;
        let cancel_timeout = self.cancel_timeout;
        let mut check = || {
            if run.closing.load(Ordering::Relaxed) {
                bail!("shutting down RKLLM worker");
            }
            if run.cancelled.load(Ordering::Relaxed) {
                let since = stopping.get_or_insert_with(Instant::now);
                if since.elapsed() >= cancel_timeout {
                    bail!("RKLLM worker cancellation timed out");
                }
            }
            Ok(())
        };
        self.send(
            &Request::Run {
                prompt: run.prompt.to_owned(),
                max_new_tokens: run.max_new_tokens,
                sampling: run.sampling.clone(),
            },
            &mut check,
        )?;
        // After a stop token, the runtime must also return within the grace
        // period even if the caller hasn't dropped its Generation.
        let mut stop_sent = None::<Instant>;
        loop {
            let mut bounded = || {
                check()?;
                if stop_sent.is_some_and(|since| since.elapsed() >= ABORT_TIMEOUT) {
                    bail!("RKLLM worker did not stop after abort");
                }
                Ok(())
            };
            match self.recv(&mut bounded)? {
                Response::Token { id, text } => {
                    let keep_going = on_token(id, &text) == Step::Continue;
                    self.send(&Request::Step(keep_going), &mut bounded)?;
                    if !keep_going {
                        stop_sent.get_or_insert_with(Instant::now);
                    }
                }
                Response::Done(tokens) => return Ok(tokens),
                Response::Error(err) => bail!(err),
                _ => {
                    self.failed = Some("unexpected worker generation response".into());
                    bail!("unexpected worker generation response");
                }
            }
        }
    }

    fn clear_history(&mut self, cancelled: &AtomicBool, closing: &AtomicBool) -> Result<()> {
        let mut stopping = None;
        let cancel_timeout = self.cancel_timeout;
        let mut check = || {
            if closing.load(Ordering::Relaxed) {
                bail!("shutting down RKLLM worker");
            }
            if cancelled.load(Ordering::Relaxed) {
                let since = stopping.get_or_insert_with(Instant::now);
                if since.elapsed() >= cancel_timeout {
                    bail!("RKLLM worker cache clear cancellation timed out");
                }
            }
            Ok(())
        };
        self.send(&Request::Clear, &mut check)?;
        match self.recv(&mut check)? {
            Response::Cleared => Ok(()),
            Response::Error(err) => bail!(err),
            _ => {
                self.failed = Some("unexpected worker cache response".into());
                bail!("unexpected worker cache response");
            }
        }
    }

    fn check_health(&mut self) -> Result<()> {
        if let Some(reason) = &self.failed {
            bail!("{reason}");
        }
        if let Some(status) = self.child.try_wait().context("check RKLLM worker")? {
            bail!("RKLLM worker exited: {status}");
        }
        Ok(())
    }
}

impl Drop for ProcessBackend {
    fn drop(&mut self) {
        // Never run vendor destructors in the host. A stuck worker is killed,
        // then reaped on this dedicated supervisor thread, not on Tokio.
        if let Err(err) = self.child.kill()
            && err.kind() != std::io::ErrorKind::InvalidInput
        {
            log::warn!("rkllm: killing worker: {err}");
        }
        if let Err(err) = self.child.wait() {
            log::warn!("rkllm: reaping worker: {err}");
        }
    }
}

/// Call at the very start of a host binary, before starting Tokio or services.
/// An internal worker exits here; ordinary invocations return immediately.
pub fn run_worker_if_requested() {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(WORKER_ARG)) {
        return;
    }
    // Worker mode bypasses the host's normal logging/service initialization.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let result = worker_with(crate::rkllm::RkllmBackend::load);
    if let Err(err) = &result {
        eprintln!("rkllm worker: {err:#}");
    }
    // The loaded backend is deliberately forgotten by worker_with. _exit
    // avoids native static destructors too; the OS reclaims model resources.
    let code = if result.is_ok() { 0 } else { 1 };
    #[cfg(target_os = "linux")]
    rustix::runtime::exit_group(code);
    #[cfg(not(target_os = "linux"))]
    std::process::exit(code);
}

fn worker_with<B: Backend>(load: impl FnOnce(&LlmConfig) -> Result<B>) -> Result<()> {
    // The spawning host supplies a duplex Unix socket as stdin. Duplicate
    // the descriptor with the safe ownership API; ordinary stdin will fail
    // the socket setup below rather than being used as a protocol channel.
    let socket = UnixStream::from(std::io::stdin().as_fd().try_clone_to_owned()?);
    let mut channel = Channel::new(socket)?;
    let Request::Init { cfg, parent } = channel.recv(&mut || Ok(()))? else {
        bail!("expected worker initialization");
    };
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{Signal, getppid, set_parent_process_death_signal};
        set_parent_process_death_signal(Some(Signal::KILL))?;
        if getppid().map(|pid| pid.as_raw_nonzero().get()) != i32::try_from(parent).ok() {
            bail!("RKLLM worker parent exited before initialization");
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = parent;
    let mut backend = match load(&cfg) {
        Ok(backend) => std::mem::ManuallyDrop::new(backend),
        Err(err) => {
            channel.send(&Response::Error(format!("{err:#}")), &mut || Ok(()))?;
            return Ok(());
        }
    };
    channel.send(&Response::Ready, &mut || Ok(()))?;
    let never = AtomicBool::new(false);
    loop {
        match channel.recv(&mut || Ok(()))? {
            Request::Clear => {
                let response = match backend.clear_history(&never, &never) {
                    Ok(()) => Response::Cleared,
                    Err(err) => Response::Error(format!("{err:#}")),
                };
                channel.send(&response, &mut || Ok(()))?;
            }
            Request::Run {
                prompt,
                max_new_tokens,
                sampling,
            } => {
                let mut callback_error = None;
                let result = backend.run(
                    Run {
                        prompt: &prompt,
                        max_new_tokens,
                        sampling: &sampling,
                        cancelled: &never,
                        closing: &never,
                    },
                    &mut |id, text| {
                        let result = (|| {
                            channel.send(
                                &Response::Token {
                                    id,
                                    text: text.to_owned(),
                                },
                                &mut || Ok(()),
                            )?;
                            match channel.recv(&mut || Ok(()))? {
                                Request::Step(true) => Ok(Step::Continue),
                                Request::Step(false) => Ok(Step::Stop),
                                _ => bail!("expected worker token acknowledgement"),
                            }
                        })();
                        match result {
                            Ok(step) => step,
                            Err(err) => {
                                callback_error = Some(err);
                                Step::Stop
                            }
                        }
                    },
                );
                if let Some(err) = callback_error {
                    return Err(err);
                }
                let response = match result {
                    Ok(tokens) => Response::Done(tokens),
                    Err(err) => Response::Error(format!("{err:#}")),
                };
                channel.send(&response, &mut || Ok(()))?;
            }
            _ => bail!("unexpected worker request"),
        }
    }
}

#[cfg(test)]
#[path = "process_test.rs"]
pub(crate) mod process_test;
