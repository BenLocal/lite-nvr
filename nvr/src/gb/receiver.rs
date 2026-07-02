//! The media-receive seam. The bridge orchestration depends only on the
//! `MediaReceiver` trait; the real impl wraps ZLM's `RtpServer`, so pull/teardown
//! logic is testable without ZLM.

use gb28181::Transport;

/// A live receiver ZLM opened for one stream. Kept alive for the session; drop
/// releases the underlying port.
pub trait ReceiverHandle: Send + Sync {
    /// UDP port the device must send its PS/RTP to.
    fn port(&self) -> u16;
}

/// Opens a media receiver for a stream id.
pub trait MediaReceiver: Send + Sync {
    fn open(
        &self,
        stream_id: &str,
        transport: Transport,
    ) -> anyhow::Result<Box<dyn ReceiverHandle>>;
}

/// Real receiver: a ZLM `RtpServer` that ingests PS/RTP into a ZLM stream named
/// `stream_id`. Only UDP (tcp_mode 0) is exercised in P1-3.
pub struct ZlmRtpReceiver;

struct ZlmReceiverHandle {
    // Field order matters only for readability; Drop on RtpServer releases the port.
    server: rszlm::server::RtpServer,
}

// SAFETY: `RtpServer` wraps a raw `mk_rtp_server` pointer, a shared_ptr handle
// over ZLM's own thread-safe C API. We only call `new`, `bind_port`, and `Drop`
// (mk_rtp_server_release) — all serialized internally by ZLM — and never alias
// the pointer outside this handle. The bridge stores it in an Arc-shared map
// touched from the ZLM hook thread and tokio, so it must be Send + Sync; rszlm
// just declines to assert it. Sound for our usage.
unsafe impl Send for ZlmReceiverHandle {}
unsafe impl Sync for ZlmReceiverHandle {}

impl ReceiverHandle for ZlmReceiverHandle {
    fn port(&self) -> u16 {
        self.server.bind_port()
    }
}

impl MediaReceiver for ZlmRtpReceiver {
    fn open(
        &self,
        stream_id: &str,
        transport: Transport,
    ) -> anyhow::Result<Box<dyn ReceiverHandle>> {
        let tcp_mode = match transport {
            Transport::Udp => 0,
            // TCP media is deferred (P1-3 is UDP-only); reject explicitly rather
            // than silently mis-binding.
            Transport::TcpPassive | Transport::TcpActive => {
                return Err(anyhow::anyhow!(
                    "gb28181: TCP media transport not supported yet"
                ));
            }
        };
        // port 0 = let ZLM pick a free UDP port; bind_port() reports it.
        let server = rszlm::server::RtpServer::new(0, tcp_mode, stream_id);
        Ok(Box::new(ZlmReceiverHandle { server }))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::sync::Mutex;

    /// Test receiver: hands out deterministic ports and records opened streams.
    #[derive(Default)]
    pub struct FakeReceiver {
        pub opened: Mutex<Vec<String>>,
        next_port: Mutex<u16>,
    }

    pub struct FakeHandle {
        port: u16,
    }

    impl ReceiverHandle for FakeHandle {
        fn port(&self) -> u16 {
            self.port
        }
    }

    impl MediaReceiver for FakeReceiver {
        fn open(
            &self,
            stream_id: &str,
            _transport: Transport,
        ) -> anyhow::Result<Box<dyn ReceiverHandle>> {
            self.opened.lock().unwrap().push(stream_id.to_string());
            let mut p = self.next_port.lock().unwrap();
            *p = if *p == 0 { 40000 } else { *p + 2 };
            Ok(Box::new(FakeHandle { port: *p }))
        }
    }

    #[test]
    fn fake_hands_out_ports_and_records() {
        let r = FakeReceiver::default();
        let h = r.open("cam1", Transport::Udp).unwrap();
        assert_eq!(h.port(), 40000);
        assert_eq!(r.opened.lock().unwrap().as_slice(), &["cam1".to_string()]);
    }
}
