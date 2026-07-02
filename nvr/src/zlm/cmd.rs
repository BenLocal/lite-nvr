//! The ZLM control worker: a dedicated OS thread owns every `RtpServer` (keyed
//! by stream id) and runs ZLM's synchronous control FFI. Async callers use the
//! `ZlmControl` facade (a cheap-clone tokio-mpsc sender) and await a oneshot
//! reply. Keeping the `RtpServer`s on one thread means they never cross a thread
//! boundary — no `unsafe impl Send` anywhere.

use std::collections::HashMap;
use std::net::SocketAddr;

use gb28181::Transport;
use rszlm::server::{RtpInfo, RtpServer, RtpServerTcpMode};
use tokio::sync::{mpsc, oneshot};

/// A command for the ZLM worker thread. Variants that produce a result carry a
/// oneshot `reply`.
pub enum ZlmCmd {
    /// Create an `RtpServer` in `mode` for `stream_id`; reply with the bound port.
    OpenRtp {
        stream_id: String,
        mode: RtpServerTcpMode,
        reply: oneshot::Sender<anyhow::Result<u16>>,
    },
    /// TCP-active: connect the existing server for `stream_id` out to `remote`.
    ConnectRtp {
        stream_id: String,
        remote: SocketAddr,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    /// Drop the `RtpServer` for `stream_id` (releases the port). Fire-and-forget.
    CloseRtp { stream_id: String },
    /// Query live RTP receive info for `app`/`stream`.
    GetRtpInfo {
        app: String,
        stream: String,
        reply: oneshot::Sender<Option<RtpInfo>>,
    },
}

impl std::fmt::Debug for ZlmCmd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZlmCmd::OpenRtp { stream_id, .. } => write!(f, "OpenRtp({stream_id})"),
            ZlmCmd::ConnectRtp {
                stream_id, remote, ..
            } => write!(f, "ConnectRtp({stream_id},{remote})"),
            ZlmCmd::CloseRtp { stream_id } => write!(f, "CloseRtp({stream_id})"),
            ZlmCmd::GetRtpInfo { app, stream, .. } => write!(f, "GetRtpInfo({app},{stream})"),
        }
    }
}

/// Map a gb transport to ZLM's RTP server TCP mode.
pub fn mode_for(transport: Transport) -> RtpServerTcpMode {
    match transport {
        Transport::Udp => RtpServerTcpMode::Disabled,
        Transport::TcpPassive => RtpServerTcpMode::Passive,
        Transport::TcpActive => RtpServerTcpMode::Active,
    }
}

/// Cheap-clone async facade over the worker's command channel.
#[derive(Clone)]
pub struct ZlmControl {
    tx: mpsc::Sender<ZlmCmd>,
}

impl ZlmControl {
    /// Start the worker thread and return the facade.
    pub fn spawn() -> Self {
        let (tx, mut rx) = mpsc::channel::<ZlmCmd>(1024);
        std::thread::Builder::new()
            .name("zlm-rtp".into())
            .spawn(move || {
                let mut servers: HashMap<String, RtpServer> = HashMap::new();
                // `blocking_recv` drains the tokio channel from a plain OS thread.
                while let Some(cmd) = rx.blocking_recv() {
                    handler_zlm_cmd(cmd, &mut servers);
                }
            })
            .expect("spawn zlm-rtp worker thread");
        Self { tx }
    }

    #[cfg(test)]
    pub fn for_test(tx: mpsc::Sender<ZlmCmd>) -> Self {
        Self { tx }
    }

    pub async fn open_rtp(&self, stream_id: &str, mode: RtpServerTcpMode) -> anyhow::Result<u16> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ZlmCmd::OpenRtp {
                stream_id: stream_id.to_string(),
                mode,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("zlm worker gone"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("zlm worker dropped reply"))?
    }

    pub async fn connect_rtp(&self, stream_id: &str, remote: SocketAddr) -> anyhow::Result<()> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(ZlmCmd::ConnectRtp {
                stream_id: stream_id.to_string(),
                remote,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("zlm worker gone"))?;
        rx.await
            .map_err(|_| anyhow::anyhow!("zlm worker dropped reply"))?
    }

    /// Fire-and-forget: safe to call from a `Drop` (never blocks / awaits).
    pub fn close_rtp(&self, stream_id: &str) {
        if let Err(e) = self.tx.try_send(ZlmCmd::CloseRtp {
            stream_id: stream_id.to_string(),
        }) {
            log::warn!("gb28181: close_rtp({stream_id}) not queued: {e}");
        }
    }

    pub async fn rtp_info(&self, app: &str, stream: &str) -> Option<RtpInfo> {
        let (reply, rx) = oneshot::channel();
        if self
            .tx
            .send(ZlmCmd::GetRtpInfo {
                app: app.to_string(),
                stream: stream.to_string(),
                reply,
            })
            .await
            .is_err()
        {
            return None;
        }
        rx.await.ok().flatten()
    }
}

/// Execute one command against the worker's `RtpServer` table. Runs only on the
/// worker thread.
pub(crate) fn handler_zlm_cmd(cmd: ZlmCmd, servers: &mut HashMap<String, RtpServer>) {
    match cmd {
        ZlmCmd::OpenRtp {
            stream_id,
            mode,
            reply,
        } => {
            // port 0 = let ZLM pick a free port; bind_port() reports it.
            let server = RtpServer::new(0, mode, &stream_id);
            let port = server.bind_port();
            servers.insert(stream_id, server);
            let _ = reply.send(if port == 0 {
                Err(anyhow::anyhow!("rtp server failed to bind a port"))
            } else {
                Ok(port)
            });
        }
        ZlmCmd::ConnectRtp {
            stream_id,
            remote,
            reply,
        } => {
            let Some(server) = servers.get(&stream_id) else {
                let _ = reply.send(Err(anyhow::anyhow!(
                    "connect: no rtp server for {stream_id}"
                )));
                return;
            };
            // The connect result arrives asynchronously on a ZLM thread via this
            // callback (FnMut) — move the reply in and fire it once.
            let mut reply = Some(reply);
            server.connect(
                &remote.ip().to_string(),
                remote.port(),
                move |code, msg, _| {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(if code == 0 {
                            Ok(())
                        } else {
                            Err(anyhow::anyhow!("rtp connect failed: code={code} {msg}"))
                        });
                    }
                },
            );
        }
        ZlmCmd::CloseRtp { stream_id } => {
            servers.remove(&stream_id); // Drop releases the port.
        }
        ZlmCmd::GetRtpInfo { app, stream, reply } => {
            let _ = reply.send(rszlm::server::rtp_get_info(&app, &stream));
        }
    }
}

#[cfg(test)]
#[path = "cmd_test.rs"]
mod cmd_test;
