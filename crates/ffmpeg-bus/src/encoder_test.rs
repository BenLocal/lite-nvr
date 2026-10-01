use std::path::Path;
use std::time::Duration;

use crate::decoder::{Decoder, DecoderTask};
use crate::encoder::{Encoder, EncoderTask, Settings};
use crate::input::AvInput;
use crate::packet::RawPacketCmd;

/// A bus-managed (auto-stop) encoder stops as soon as its last output
/// subscriber leaves, while frames are still arriving.
#[tokio::test(flavor = "multi_thread")]
async fn test_auto_stop_encoder_stops_without_subscribers() -> anyhow::Result<()> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test.mp4");
    if !path.exists() {
        return Ok(());
    }
    let mut input = AvInput::new(path.to_str().unwrap_or_default(), None, None)?;
    let stream = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video stream"))?
        .clone();
    let mut packets = Vec::new();
    while let Some(p) = input.read_packet() {
        if p.index() == stream.index() {
            packets.push(RawPacketCmd::Data(p));
        }
    }

    // Paced source: one packet every 20ms (~1s for the 50 frames).
    let (tx, rx) = tokio::sync::broadcast::channel(4096);
    let feed = tokio::spawn(async move {
        for p in packets {
            let _ = tx.send(p);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let _ = tx.send(RawPacketCmd::EOF);
    });
    let decoder = DecoderTask::new();
    let frames = decoder.subscribe(true);
    decoder.start(Decoder::new(&stream)?, rx).await;

    let settings = Settings {
        width: 160,
        height: 120,
        ..Settings::default()
    };
    let task = EncoderTask::new_auto_stop();
    let mut out = task.subscribe();
    task.start(Encoder::new(&stream, settings, None)?, frames, false)
        .await;

    let first = tokio::time::timeout(Duration::from_secs(5), out.recv()).await??;
    assert!(matches!(first, RawPacketCmd::Data(_)));
    drop(out);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while task.is_running() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !task.is_running(),
        "encoder kept running with no subscribers"
    );
    assert!(!feed.is_finished(), "must stop before the source ends");
    assert!(task.try_subscribe().is_none());
    Ok(())
}
