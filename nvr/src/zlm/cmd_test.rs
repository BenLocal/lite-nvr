use super::*;
use gb28181::Transport;

#[test]
fn mode_for_maps_transports() {
    assert!(matches!(
        mode_for(Transport::Udp),
        rszlm::server::RtpServerTcpMode::Disabled
    ));
    assert!(matches!(
        mode_for(Transport::TcpPassive),
        rszlm::server::RtpServerTcpMode::Passive
    ));
    assert!(matches!(
        mode_for(Transport::TcpActive),
        rszlm::server::RtpServerTcpMode::Active
    ));
}

// The facade builds the right command and returns the worker's reply, without
// touching ZLM: we stand in for the worker by draining the channel ourselves.
#[tokio::test]
async fn open_rtp_sends_command_and_returns_reply() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ZlmCmd>(4);
    let ctrl = ZlmControl::for_test(tx);
    let task = tokio::spawn(async move {
        ctrl.open_rtp("cam1", rszlm::server::RtpServerTcpMode::Disabled)
            .await
    });

    match rx.recv().await.expect("cmd") {
        ZlmCmd::OpenRtp {
            stream_id, reply, ..
        } => {
            assert_eq!(stream_id, "cam1");
            reply.send(Ok(41000)).unwrap();
        }
        other => panic!("unexpected cmd: {other:?}"),
    }
    assert_eq!(task.await.unwrap().unwrap(), 41000);
}

#[tokio::test]
async fn rtp_info_sends_command_and_returns_reply() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ZlmCmd>(4);
    let ctrl = ZlmControl::for_test(tx);
    let task = tokio::spawn(async move { ctrl.rtp_info("rtp", "cam1").await });
    match rx.recv().await.expect("cmd") {
        ZlmCmd::GetRtpInfo { app, stream, reply } => {
            assert_eq!((app.as_str(), stream.as_str()), ("rtp", "cam1"));
            reply.send(None).unwrap();
        }
        other => panic!("unexpected cmd: {other:?}"),
    }
    assert!(task.await.unwrap().is_none());
}
