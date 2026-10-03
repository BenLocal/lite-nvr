use std::time::Duration;

use crate::input::{AvInput, AvInputTask};
use crate::packet::RawPacketCmd;

/// In lossless mode a source that produces far more than the channel capacity
/// in a burst must not make a slow subscriber lag: every packet arrives.
#[tokio::test(flavor = "multi_thread")]
async fn test_lossless_reader_never_lags() -> anyhow::Result<()> {
    crate::init()?;
    // 300s @ 25fps of tiny raw frames = 7500 packets, > PACKET_CHAN_CAP (4096).
    let input = AvInput::new(
        "testsrc=duration=300:size=16x16:rate=25",
        Some("lavfi"),
        None,
    )?;
    let task = AvInputTask::new();
    task.set_lossless();
    let mut rx = task.subscribe();
    task.start(input).await;
    // Let the reader race ahead of us.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let count = tokio::time::timeout(Duration::from_secs(60), async move {
        let mut n = 0usize;
        loop {
            match rx.recv().await? {
                RawPacketCmd::Data(_) => n += 1,
                RawPacketCmd::EOF => return anyhow::Ok(n),
            }
        }
    })
    .await??;
    assert_eq!(count, 7500);
    Ok(())
}

/// A raw video input reports avg_frame_rate 0/0; the stream must still carry
/// the real frame rate so the encoder is configured correctly.
#[test]
fn test_rawvideo_stream_frame_rate() -> anyhow::Result<()> {
    crate::init()?;
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".test_media");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("input_rate.yuv");
    std::fs::write(&path, vec![0u8; 64 * 48 * 3 / 2 * 3])?;
    let options = ffmpeg_next::Dictionary::from_iter([
        ("video_size", "64x48"),
        ("pixel_format", "yuv420p"),
        ("framerate", "10"),
    ]);
    let input = AvInput::new(
        path.to_str().unwrap_or_default(),
        Some("rawvideo"),
        Some(options),
    )?;
    let video = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video stream"))?;
    assert_eq!(video.rate(), ffmpeg_next::Rational(10, 1));
    Ok(())
}

/// Without lossless mode (live sources) the reader never waits: a subscriber
/// that stops reading just lags, while the others keep receiving everything.
#[tokio::test(flavor = "multi_thread")]
async fn test_live_reader_does_not_wait_for_stalled_subscriber() -> anyhow::Result<()> {
    crate::init()?;
    let input = AvInput::new(
        "testsrc=duration=300:size=16x16:rate=25",
        Some("lavfi"),
        None,
    )?;
    let task = AvInputTask::new();
    let _stalled = task.subscribe(); // never read
    let mut rx = task.subscribe();
    task.start(input).await;

    let count = tokio::time::timeout(Duration::from_secs(60), async move {
        let mut n = 0usize;
        loop {
            match rx.recv().await {
                Ok(RawPacketCmd::Data(_)) => n += 1,
                Ok(RawPacketCmd::EOF) => return anyhow::Ok(n),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => return Err(e.into()),
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("reader blocked on the stalled subscriber"))??;
    assert!(count > 0);
    assert!(!task.is_lossless());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_lossless_eof_does_not_evict_last_full_queue() -> anyhow::Result<()> {
    crate::init()?;
    let input = AvInput::new(
        "testsrc=duration=4096:size=16x16:rate=1",
        Some("lavfi"),
        None,
    )?;
    let task = AvInputTask::new();
    task.set_lossless();
    let mut rx = task.subscribe();
    task.start(input).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while task.raw_chan.len() < AvInputTask::PACKET_CHAN_CAP {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await?;
    // Give the producer time to encounter EOF while every slot is occupied.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let count = tokio::time::timeout(Duration::from_secs(5), async {
        let mut count = 0;
        loop {
            match rx.recv().await? {
                RawPacketCmd::Data(_) => count += 1,
                RawPacketCmd::EOF => return anyhow::Ok(count),
            }
        }
    })
    .await??;
    assert_eq!(count, 4096);
    Ok(())
}
