use std::path::Path;
use std::time::Duration;

use futures::StreamExt;

use crate::input::AvInput;
use crate::output::AvOutputStream;

/// A slow reader must not lose muxed bytes: the writer waits for room
/// instead of dropping chunks (a lost chunk corrupts the byte stream).
#[tokio::test(flavor = "multi_thread")]
async fn test_mux_stream_slow_reader_loses_nothing() -> anyhow::Result<()> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test.mp4");
    if !path.exists() {
        return Ok(());
    }
    let mut input = AvInput::new(path.to_str().unwrap_or_default(), None, None)?;
    let video = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video"))?
        .clone();
    let mut packets = Vec::new();
    while let Some(p) = input.read_packet() {
        if p.index() == video.index() {
            packets.push(p);
        }
    }

    let mut stream = AvOutputStream::new("h264")?;
    stream.add_stream(&video)?;
    let (mut writer, mut reader) = stream.into_split();

    // 10 passes over the 50 packets: 500 chunks, more than the 256-message
    // channel, written as fast as possible.
    const PASSES: usize = 10;
    let writer = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        for _ in 0..PASSES {
            for p in &packets {
                writer.write_packet(p.clone())?;
            }
        }
        writer.finish()
    });

    let mut received = 0usize;
    while let Some(_chunk) = tokio::time::timeout(Duration::from_secs(20), reader.next()).await? {
        received += 1;
        tokio::time::sleep(Duration::from_millis(1)).await; // slow reader
    }
    writer.await??;
    assert_eq!(received, PASSES * 50, "every muxed chunk arrives");
    Ok(())
}
