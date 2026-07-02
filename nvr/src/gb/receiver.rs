//! The media-receive seam. The bridge depends only on the `MediaReceiver` trait,
//! so pull/teardown logic is testable without ZLM. The real impl drives the
//! `ZlmControl` worker; the worker owns the `RtpServer`, so nothing here holds a
//! raw ZLM pointer — no `unsafe impl Send`.

use std::net::SocketAddr;

use async_trait::async_trait;
use gb28181::Transport;

use crate::zlm::cmd::{ZlmControl, mode_for};

/// A live receiver ZLM opened for one stream. Dropping it releases the port.
#[async_trait]
pub trait ReceiverHandle: Send {
    /// Port the device must send its PS/RTP to (UDP) or connect to (TCP passive).
    fn port(&self) -> u16;
    /// TCP-active only: connect out to the device's media addr (from the SDP
    /// answer). No-op / unused for UDP and TCP-passive.
    async fn connect(&self, remote: SocketAddr) -> anyhow::Result<()>;
}

/// Opens a media receiver for a stream id.
#[async_trait]
pub trait MediaReceiver: Send + Sync {
    async fn open(
        &self,
        stream_id: &str,
        transport: Transport,
    ) -> anyhow::Result<Box<dyn ReceiverHandle>>;
}

/// Real receiver: drives the `ZlmControl` worker to create/connect/close
/// `RtpServer`s that publish `stream_id` under ZLM's `rtp` app.
pub struct ZlmRtpReceiver {
    control: ZlmControl,
}

impl ZlmRtpReceiver {
    pub fn new(control: ZlmControl) -> Self {
        Self { control }
    }
}

struct ZlmReceiverHandle {
    stream_id: String,
    port: u16,
    control: ZlmControl,
}

#[async_trait]
impl ReceiverHandle for ZlmReceiverHandle {
    fn port(&self) -> u16 {
        self.port
    }
    async fn connect(&self, remote: SocketAddr) -> anyhow::Result<()> {
        self.control.connect_rtp(&self.stream_id, remote).await
    }
}

impl Drop for ZlmReceiverHandle {
    fn drop(&mut self) {
        self.control.close_rtp(&self.stream_id); // fire-and-forget, releases the port
    }
}

#[async_trait]
impl MediaReceiver for ZlmRtpReceiver {
    async fn open(
        &self,
        stream_id: &str,
        transport: Transport,
    ) -> anyhow::Result<Box<dyn ReceiverHandle>> {
        let port = self
            .control
            .open_rtp(stream_id, mode_for(transport))
            .await?;
        Ok(Box::new(ZlmReceiverHandle {
            stream_id: stream_id.to_string(),
            port,
            control: self.control.clone(),
        }))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;

    /// Test receiver: hands out deterministic ports and records (stream_id,
    /// transport) per open and the remote of each active connect. All state is
    /// behind `Arc` so a test can `clone()` the receiver, move one copy into the
    /// bridge, and read the recordings through the other.
    #[derive(Default, Clone)]
    pub struct FakeReceiver {
        pub opened: Arc<Mutex<Vec<(String, Transport)>>>,
        pub connected: Arc<Mutex<Vec<SocketAddr>>>,
        next_port: Arc<Mutex<u16>>,
    }

    pub struct FakeHandle {
        port: u16,
        connected: Arc<Mutex<Vec<SocketAddr>>>,
    }

    #[async_trait]
    impl ReceiverHandle for FakeHandle {
        fn port(&self) -> u16 {
            self.port
        }
        async fn connect(&self, remote: SocketAddr) -> anyhow::Result<()> {
            self.connected.lock().unwrap().push(remote);
            Ok(())
        }
    }

    #[async_trait]
    impl MediaReceiver for FakeReceiver {
        async fn open(
            &self,
            stream_id: &str,
            transport: Transport,
        ) -> anyhow::Result<Box<dyn ReceiverHandle>> {
            self.opened
                .lock()
                .unwrap()
                .push((stream_id.to_string(), transport));
            let mut p = self.next_port.lock().unwrap();
            *p = if *p == 0 { 40000 } else { *p + 2 };
            Ok(Box::new(FakeHandle {
                port: *p,
                connected: self.connected.clone(),
            }))
        }
    }

    #[tokio::test]
    async fn fake_records_open_and_connect() {
        let r = FakeReceiver::default();
        let h = r.open("cam1", Transport::TcpActive).await.unwrap();
        assert_eq!(h.port(), 40000);
        h.connect("1.2.3.4:5000".parse().unwrap()).await.unwrap();
        assert_eq!(
            r.opened.lock().unwrap().as_slice(),
            &[("cam1".to_string(), Transport::TcpActive)]
        );
        assert_eq!(
            r.connected.lock().unwrap().as_slice(),
            &["1.2.3.4:5000".parse().unwrap()]
        );
    }
}
