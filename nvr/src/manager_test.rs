use super::*;

#[tokio::test]
async fn concurrent_replacements_cancel_superseded_worker() {
    let id = "concurrent-replacements";
    let gate = Arc::new(tokio::sync::Notify::new());
    let old_cancel = CancellationToken::new();
    let old_gate = gate.clone();
    let old_token = old_cancel.clone();
    let handle = tokio::spawn(async move {
        old_token.cancelled().await;
        old_gate.notified().await;
    });
    PIPE_MANAGER.write().await.insert(
        id.into(),
        Entry::Task {
            cancel: old_cancel.clone(),
            handle,
        },
    );
    let first_cancel = CancellationToken::new();
    let first_token = first_cancel.clone();
    let first = tokio::spawn(async move {
        upsert_entry(
            id,
            move || {
                let token = first_token.clone();
                Entry::Task {
                    cancel: first_token,
                    handle: tokio::spawn(async move {
                        token.cancelled().await;
                    }),
                }
            },
            true,
        )
        .await
        .unwrap();
    });
    old_cancel.cancelled().await;
    let second_cancel = CancellationToken::new();
    let second_token = second_cancel.clone();
    let second = tokio::spawn(async move {
        upsert_entry(
            id,
            move || {
                let token = second_token.clone();
                Entry::Task {
                    cancel: second_token,
                    handle: tokio::spawn(async move {
                        token.cancelled().await;
                    }),
                }
            },
            true,
        )
        .await
        .unwrap();
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    gate.notify_one();
    first.await.unwrap();
    second.await.unwrap();
    let superseded_stopped = first_cancel.is_cancelled();
    // Clean up even when run against the original buggy implementation.
    first_cancel.cancel();
    second_cancel.cancel();
    remove_pipe(id).await.unwrap();
    assert!(
        superseded_stopped,
        "replacement lost its JoinHandle without being cancelled"
    );
}

async fn transport_stream_bytes() -> Vec<u8> {
    use ffmpeg_bus::bus::{
        Bus, InputConfig as FbInput, OutputAvType, OutputConfig as FbOutput, OutputDest,
    };
    use futures::StreamExt;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../e2e/test.mp4");
    assert!(
        path.is_file(),
        "e2e/test.mp4 is required for this network regression test"
    );
    let bus = Bus::new_deferred("network-regression-ts");
    bus.add_input(
        FbInput::File {
            path: path.to_string_lossy().into_owned(),
        },
        None,
    )
    .await
    .unwrap();
    let (_, mut stream) = bus
        .add_output(FbOutput::new(
            "ts".into(),
            OutputAvType::Video,
            OutputDest::Mux {
                format: "mpegts".into(),
            },
        ))
        .await
        .unwrap();
    bus.start().await.unwrap();
    let mut bytes = Vec::new();
    while let Some(Some(frame)) = stream.next().await {
        bytes.extend_from_slice(&frame.data);
    }
    bytes
}

async fn check_network_device_recovery(first_open_fails: bool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::AsyncWriteExt;
    let bytes = transport_stream_bytes().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let count = connections.clone();
    let connected = Arc::new(tokio::sync::Notify::new());
    let ready = connected.clone();
    let release = Arc::new(tokio::sync::Notify::new());
    let first_release = release.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut client, _) = listener.accept().await.unwrap();
            let n = count.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                ready.notify_one();
                first_release.notified().await;
            }
            if n != 0 || !first_open_fails {
                let _ = client.write_all(&bytes).await;
            }
            let _ = client.shutdown().await;
        }
    });
    static ZLM_INIT: std::sync::Once = std::sync::Once::new();
    ZLM_INIT.call_once(|| {
        rszlm::init::EnvInitBuilder::default()
            .log_level(0)
            .log_mask(0)
            .thread_num(2)
            .build();
    });
    let device = DeviceInfo {
        id: format!("network-reconnect-regression-{first_open_fails}"),
        name: "Network regression".into(),
        input_type: "net".into(),
        input_value: format!("tcp://{addr}"),
        description: String::new(),
        include_audio: false,
        record: false,
        config: Default::default(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let hub = Box::leak(Box::new(DetectHub::new_for_test(
        vec![],
        std::path::PathBuf::new(),
        500,
    )));
    crate::init::device::ensure_device_pipe(hub, &device)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), connected.notified())
        .await
        .unwrap();
    assert!(
        get_pipe(&device.id).await.is_some(),
        "reconnecting devices must expose their current pipe"
    );
    release.notify_one();
    let reconnected = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while connections.load(Ordering::SeqCst) < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(3), remove_pipe(&device.id))
        .await
        .unwrap()
        .unwrap();
    let stopped_count = connections.load(Ordering::SeqCst);
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    let count_after_remove = connections.load(Ordering::SeqCst);
    server.abort();
    reconnected.expect("network device never reconnected after EOF");
    assert_eq!(
        count_after_remove, stopped_count,
        "removed device kept reconnecting"
    );
    assert!(get_pipe(&device.id).await.is_none());
}

#[tokio::test]
async fn remove_waits_for_replacement_and_stops_the_new_source() {
    let id = "remove-during-replacement";
    let gate = Arc::new(tokio::sync::Notify::new());
    let old_cancel = CancellationToken::new();
    let old_gate = gate.clone();
    let old_token = old_cancel.clone();
    let handle = tokio::spawn(async move {
        old_token.cancelled().await;
        old_gate.notified().await;
    });
    PIPE_MANAGER.write().await.insert(
        id.into(),
        Entry::Task {
            cancel: old_cancel.clone(),
            handle,
        },
    );
    let new_cancel = CancellationToken::new();
    let new_token = new_cancel.clone();
    let replacement = tokio::spawn(async move {
        upsert_entry(
            id,
            move || {
                let token = new_token.clone();
                Entry::Task {
                    cancel: new_token,
                    handle: tokio::spawn(async move {
                        token.cancelled().await;
                    }),
                }
            },
            true,
        )
        .await
        .unwrap();
    });
    old_cancel.cancelled().await;
    let removal = tokio::spawn(async move {
        remove_pipe(id).await.unwrap();
    });
    tokio::task::yield_now().await;
    gate.notify_one();
    replacement.await.unwrap();
    removal.await.unwrap();
    let stopped = new_cancel.is_cancelled();
    new_cancel.cancel();
    remove_pipe(id).await.unwrap();
    assert!(stopped);
    assert_eq!(status(id).await, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn network_device_reconnects_and_remove_stops_retries() {
    check_network_device_recovery(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn network_device_retries_after_failed_initial_open() {
    check_network_device_recovery(true).await;
}
