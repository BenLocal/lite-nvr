use std::path::Path;
use std::time::Duration;

use futures::StreamExt;

use crate::input::AvInput;
use crate::output::AvOutputStream;

/// A slow reader must not lose muxed bytes: the writer waits for room
/// instead of dropping chunks (a lost chunk corrupts the byte stream).
#[tokio::test(flavor = "multi_thread")]
async fn test_mux_stream_slow_reader_loses_nothing() -> anyhow::Result<()> {
    let path = crate::test_mp4_path();
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

/// A raw-video input with big frames (lots of bytes per packet) from a file
/// written for the test.
fn big_raw_video_input() -> anyhow::Result<AvInput> {
    crate::init()?;
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".test_media");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("big_640x480.yuv");
    std::fs::write(&path, vec![0x80u8; 640 * 480 * 3 / 2 * 4])?;
    let options = ffmpeg_next::Dictionary::from_iter([
        ("video_size", "640x480"),
        ("pixel_format", "yuv420p"),
        ("framerate", "25"),
    ]);
    AvInput::new(
        path.to_str().unwrap_or_default(),
        Some("rawvideo"),
        Some(options),
    )
}

/// Accept one connection, read a little, then stop reading (keeping it open):
/// a push target that stalls, so the sender's writes eventually block.
fn stalling_reader_server() -> anyhow::Result<u16> {
    use std::io::Read;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        if let Ok((mut conn, _)) = listener.accept() {
            let mut buf = [0u8; 64 * 1024];
            let _ = conn.read(&mut buf);
            std::thread::sleep(Duration::from_secs(60));
            drop(conn);
        }
    });
    Ok(port)
}

/// A network output blocked on a peer that stopped reading is aborted by its
/// interrupt flag (what `Bus::stop` sets) instead of blocking forever.
#[tokio::test(flavor = "multi_thread")]
async fn test_network_output_write_is_interruptible() -> anyhow::Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let mut input = big_raw_video_input()?;
    let video = input
        .streams()
        .values()
        .find(|s| s.is_video())
        .ok_or_else(|| anyhow::anyhow!("no video"))?
        .clone();
    let packets: Vec<_> = std::iter::from_fn(|| input.read_packet()).collect();
    let port = stalling_reader_server()?;

    let interrupt = Arc::new(AtomicBool::new(false));
    let written = Arc::new(AtomicUsize::new(0));
    let (flag, count) = (interrupt.clone(), written.clone());
    let writer = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut out = crate::output::AvOutput::open_network(
            &format!("tcp://127.0.0.1:{port}"),
            Some("matroska"),
            None,
            flag,
        )?;
        out.add_stream(&video)?;
        out.write_header()?;
        // Keep writing (re-timestamped copies) until the socket blocks.
        for i in 0.. {
            let mut p = packets[i % packets.len()].clone();
            p.get_mut().set_pts(Some(i as i64));
            p.get_mut().set_dts(Some(i as i64));
            out.write_packet(video.index(), p)?;
            count.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    });

    // Wait until writes stop progressing: the peer's buffers are full.
    let mut last = usize::MAX;
    loop {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let now = written.load(Ordering::Relaxed);
        if now == last && now > 0 {
            break;
        }
        last = now;
    }
    assert!(
        !writer.is_finished(),
        "writer should be blocked on the stalled peer"
    );
    interrupt.store(true, Ordering::Relaxed);
    let res = tokio::time::timeout(Duration::from_secs(3), writer)
        .await
        .map_err(|_| anyhow::anyhow!("blocked network write was not interrupted"))??;
    assert!(res.is_err(), "interrupted write reports an error");
    Ok(())
}
