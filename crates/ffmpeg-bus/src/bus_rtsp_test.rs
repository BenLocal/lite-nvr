//! End-to-end RTSP live tests, simulated in-process with plain FFmpeg (no
//! external server): the bus under test opens `rtsp://127.0.0.1:<port>/live`
//! with `rtsp_flags=listen`, acting as the RTSP server, and a second bus
//! publishes to it over RTSP/TCP. Two publishers are used:
//! - `test.mp4` (320x240, 10fps, 50 frames + AAC), pushed as fast as possible;
//! - a lavfi `testsrc` paced by the `realtime` filter, i.e. a true live source.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::task::JoinHandle;

use super::bus_test::{test_media, test_mp4_path};
use crate::bus::{
    Bus, EncodeConfig, InputConfig, OutputAvType, OutputConfig, OutputDest, VideoRawFrameStream,
};
use crate::frame::{RawFrameCmd, RawFrameReceiver, VideoFrame};
use crate::metadata::probe;
use crate::stream::AvStream;

/// Upper bound for any single wait in these tests.
const WAIT: Duration = Duration::from_secs(20);

/// A free local RTSP URL.
fn rtsp_url() -> anyhow::Result<String> {
    let port = std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port();
    Ok(format!("rtsp://127.0.0.1:{port}/live"))
}

/// A bus whose input is an RTSP server listening on `url`. Its first output
/// blocks until a publisher connects, so start it with [`spawn_first_output`].
/// `live: false` for publishers that push a file in a burst and tests that
/// check nothing is lost; `true` for real-time paced sources.
async fn listening_bus(url: &str, live: bool) -> anyhow::Result<Arc<Bus>> {
    let bus = Arc::new(Bus::new("rtsp-rx"));
    let options = [
        ("rtsp_flags", "listen"),
        ("rtsp_transport", "tcp"),
        ("listen_timeout", "15"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    bus.add_input_with_live(
        InputConfig::Net {
            url: url.to_string(),
        },
        Some(options),
        live,
    )
    .await?;
    Ok(bus)
}

type OutputResult = anyhow::Result<(AvStream, VideoRawFrameStream)>;

fn spawn_first_output(bus: &Arc<Bus>, output: OutputConfig) -> JoinHandle<OutputResult> {
    let bus = bus.clone();
    tokio::spawn(async move { bus.add_output(output).await })
}

async fn join_output(handle: JoinHandle<OutputResult>) -> OutputResult {
    tokio::time::timeout(WAIT, handle)
        .await
        .map_err(|_| anyhow::anyhow!("listening output never got a publisher"))??
}

fn file_input() -> InputConfig {
    InputConfig::File {
        path: test_mp4_path().to_string_lossy().into_owned(),
    }
}

/// Real-time paced synthetic camera: `secs` seconds of 320x240 @ 25fps.
fn live_input(secs: u32) -> InputConfig {
    InputConfig::Device {
        display: format!("testsrc=duration={secs}:size=320x240:rate=25,realtime"),
        format: "lavfi".to_string(),
    }
}

fn rtsp_push(url: &str) -> OutputConfig {
    OutputConfig::new(
        "rtsp_push".to_string(),
        OutputAvType::Video,
        OutputDest::Net {
            url: url.to_string(),
            format: Some("rtsp".to_string()),
        },
    )
}

/// Publish to `url`, retrying until the listener is up. Keep the returned bus
/// alive for as long as the stream should run.
async fn publish(
    url: &str,
    input: impl Fn() -> InputConfig,
    output: impl Fn() -> OutputConfig,
) -> anyhow::Result<Bus> {
    let mut last_err = None;
    for _ in 0..100 {
        let bus = Bus::new("rtsp-tx");
        bus.add_input(input(), None).await?;
        match bus.add_output(output()).await {
            Ok(_) => return Ok(bus),
            Err(e) => last_err = Some(e),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    anyhow::bail!("publisher never connected to {url}: {last_err:?}")
}

/// Publish test.mp4 (video + audio copied) as fast as possible.
async fn publish_file(url: &str) -> anyhow::Result<Bus> {
    publish(url, file_input, || rtsp_push(url).with_audio()).await
}

/// Publish the paced synthetic camera, encoded to H.264.
async fn publish_live(url: &str, secs: u32) -> anyhow::Result<Bus> {
    // lavfi is a device input: register devices first.
    crate::init()?;
    publish(
        url,
        || live_input(secs),
        || rtsp_push(url).with_encode(EncodeConfig::default()),
    )
    .await
}

fn file_output(id: &str, path: &str) -> OutputConfig {
    OutputConfig::new(
        id.to_string(),
        OutputAvType::Video,
        OutputDest::File {
            path: path.to_string(),
        },
    )
}

fn h264(width: u32, height: u32) -> EncodeConfig {
    EncodeConfig {
        width: Some(width),
        height: Some(height),
        ..Default::default()
    }
}

/// Collect a bus output stream until its EOF item (`None`) or stream end.
fn spawn_drain(mut stream: VideoRawFrameStream) -> JoinHandle<Vec<VideoFrame>> {
    tokio::spawn(async move {
        let mut frames = Vec::new();
        while let Some(item) = stream.next().await {
            match item {
                Some(frame) => frames.push(frame),
                None => break,
            }
        }
        frames
    })
}

/// Count decoded frames until EOF or channel close; lags are tolerated
/// (lossy subscriber).
fn spawn_count_decoded(mut rx: RawFrameReceiver) -> JoinHandle<usize> {
    tokio::spawn(async move {
        let mut n = 0;
        loop {
            match rx.recv().await {
                Ok(RawFrameCmd::Data(_)) => n += 1,
                Ok(RawFrameCmd::EOF) => return n,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return n,
            }
        }
    })
}

async fn finish<T>(handle: JoinHandle<T>, what: &str) -> anyhow::Result<T> {
    tokio::time::timeout(WAIT, handle)
        .await
        .map_err(|_| anyhow::anyhow!("{what} never ended (no EOF)"))?
        .map_err(Into::into)
}

/// Poll until `path` is a finished (probe-able) media file.
async fn wait_for_file(path: &str) -> anyhow::Result<()> {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if probe(path).is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("{path} was never finalized")
}

struct VideoSummary {
    width: u32,
    height: u32,
    frames: u32,
    keys: u32,
}

fn video_summary(path: &str) -> anyhow::Result<VideoSummary> {
    let info = probe(path)?;
    let video = info
        .streams
        .iter()
        .find(|s| s.codec_type == "video")
        .ok_or_else(|| anyhow::anyhow!("{path}: no video stream"))?;
    let mut input = ffmpeg_next::format::input(path)?;
    let (mut frames, mut keys) = (0, 0);
    for (stream, packet) in input.packets() {
        if stream.index() == video.index {
            frames += 1;
            keys += u32::from(packet.is_key());
        }
    }
    Ok(VideoSummary {
        width: video.width.unwrap_or(0),
        height: video.height.unwrap_or(0),
        frames,
        keys,
    })
}

fn skip_without_test_mp4() -> bool {
    let missing = !test_mp4_path().exists();
    if missing {
        log::warn!("skip: {} not found", test_mp4_path().display());
    }
    missing
}

/// Lossless transcode of a live RTSP stream lands every frame, a lossy decoded
/// subscriber sees frames, and the publisher's teardown ends the stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_rtsp_live_transcode_to_file() -> anyhow::Result<()> {
    if skip_without_test_mp4() {
        return Ok(());
    }
    let url = rtsp_url()?;
    let path = test_media("rtsp_transcode.mp4");
    std::fs::remove_file(&path).ok();

    let rx = listening_bus(&url, false).await?;
    let first = spawn_first_output(&rx, file_output("file", &path).with_encode(h264(160, 120)));
    let _tx = publish_file(&url).await?;
    let _ = join_output(first).await?;
    let decoded = spawn_count_decoded(rx.subscribe_video().await?);

    finish(decoded, "lossy decoded subscriber").await?;
    wait_for_file(&path).await?;
    let v = video_summary(&path)?;
    assert_eq!((v.width, v.height), (160, 120));
    assert_eq!(v.frames, 50, "lossless transcode keeps every frame");
    let duration = probe(&path)?.format.duration_sec.unwrap_or(0.0);
    assert!((4.5..=5.5).contains(&duration), "duration {duration}s");
    Ok(())
}

/// Copying a live RTSP stream with audio into MP4 keeps both tracks, every
/// video frame, and the source duration.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_rtsp_av_copy_to_mp4() -> anyhow::Result<()> {
    if skip_without_test_mp4() {
        return Ok(());
    }
    let url = rtsp_url()?;
    let path = test_media("rtsp_av_copy.mp4");
    std::fs::remove_file(&path).ok();

    let rx = listening_bus(&url, false).await?;
    let first = spawn_first_output(&rx, file_output("file", &path).with_audio());
    let _tx = publish_file(&url).await?;
    let _ = join_output(first).await?;

    wait_for_file(&path).await?;
    let info = probe(&path)?;
    let audio = info
        .streams
        .iter()
        .find(|s| s.codec_type == "audio")
        .ok_or_else(|| anyhow::anyhow!("audio track missing"))?;
    assert_eq!(audio.codec_name, "aac");
    let v = video_summary(&path)?;
    assert_eq!((v.width, v.height), (320, 240), "copy keeps geometry");
    assert_eq!(v.frames, 50);
    let duration = info.format.duration_sec.unwrap_or(0.0);
    assert!(
        (4.5..=5.5).contains(&duration),
        "duration {duration}s, expected ~5s"
    );
    Ok(())
}

/// Live RTSP audio transcoded (resample + remix) while video is copied.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_rtsp_audio_transcode() -> anyhow::Result<()> {
    if skip_without_test_mp4() {
        return Ok(());
    }
    let url = rtsp_url()?;
    let path = test_media("rtsp_audio_transcode.mp4");
    std::fs::remove_file(&path).ok();

    let audio = EncodeConfig {
        codec: "aac".to_string(),
        sample_rate: Some(48000),
        channels: Some(2),
        audio_bitrate: Some(96_000),
        ..Default::default()
    };
    let rx = listening_bus(&url, false).await?;
    let first = spawn_first_output(
        &rx,
        file_output("file", &path)
            .with_audio()
            .with_audio_encode(audio),
    );
    let _tx = publish_file(&url).await?;
    let _ = join_output(first).await?;

    wait_for_file(&path).await?;
    let info = probe(&path)?;
    let a = info
        .streams
        .iter()
        .find(|s| s.codec_type == "audio")
        .ok_or_else(|| anyhow::anyhow!("audio track missing"))?;
    assert_eq!((a.sample_rate, a.channels), (Some(48000), Some(2)));
    assert_eq!(video_summary(&path)?.frames, 50, "video copied untouched");
    Ok(())
}

/// Demuxed passthrough (the ZLMediaKit path): every packet of the live stream
/// arrives unmodified, starting on a keyframe, followed by EOF.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_rtsp_demuxed_passthrough() -> anyhow::Result<()> {
    if skip_without_test_mp4() {
        return Ok(());
    }
    let url = rtsp_url()?;
    let rx = listening_bus(&url, false).await?;
    let first = spawn_first_output(
        &rx,
        OutputConfig::new(
            "demuxed".to_string(),
            OutputAvType::Video,
            OutputDest::Demuxed,
        ),
    );
    let _tx = publish_file(&url).await?;
    let (av, stream) = join_output(first).await?;
    assert!(av.is_video());
    assert_eq!(av.parameters().id(), ffmpeg_next::codec::Id::H264);

    let frames = finish(spawn_drain(stream), "demuxed stream").await?;
    assert_eq!(frames.len(), 50);
    assert!(frames[0].is_key, "live stream must start on a keyframe");
    assert!(frames.iter().all(|f| !f.data.is_empty()));
    Ok(())
}

/// Several outputs of different kinds share one real-time live input: the
/// demuxed passthrough, decoded raw frames, a re-encoded packet stream, a
/// copy mux and a lossy subscriber all get data and all end with the stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_rtsp_paced_live_multi_output() -> anyhow::Result<()> {
    let url = rtsp_url()?;
    let rx = listening_bus(&url, true).await?;
    let first = spawn_first_output(
        &rx,
        OutputConfig::new(
            "demuxed".to_string(),
            OutputAvType::Video,
            OutputDest::Demuxed,
        ),
    );
    let started = Instant::now();
    let _tx = publish_live(&url, 3).await?;
    let (_, demuxed) = join_output(first).await?;
    let demuxed = spawn_drain(demuxed);

    let (_, raw) = rx
        .add_output(OutputConfig::new(
            "raw".to_string(),
            OutputAvType::Video,
            OutputDest::Raw,
        ))
        .await?;
    let raw = spawn_drain(raw);
    let (_, encoded) = rx
        .add_output(
            OutputConfig::new(
                "encoded".to_string(),
                OutputAvType::Video,
                OutputDest::Encoded,
            )
            .with_encode(h264(160, 120)),
        )
        .await?;
    let encoded = spawn_drain(encoded);
    let (_, mux) = rx
        .add_output(OutputConfig::new(
            "mux".to_string(),
            OutputAvType::Video,
            OutputDest::Mux {
                format: "h264".to_string(),
            },
        ))
        .await?;
    let mux = spawn_drain(mux);
    let lossy = spawn_count_decoded(rx.subscribe_video().await?);

    let demuxed = finish(demuxed, "demuxed").await?;
    let raw = finish(raw, "raw").await?;
    let encoded = finish(encoded, "encoded").await?;
    let mux = finish(mux, "mux").await?;
    let lossy = finish(lossy, "lossy subscriber").await?;

    // Real-time pacing: 3s of video cannot arrive in a burst.
    assert!(
        started.elapsed() >= Duration::from_secs(2),
        "source not paced"
    );
    // The first output sees the whole stream (3s @ 25fps = 75 frames).
    assert!(demuxed.len() >= 70, "demuxed frames: {}", demuxed.len());
    // Later outputs join a running stream and are lossy: how much they get
    // depends on load, so require data (and the EOF that ended each drain).
    assert!(!raw.is_empty(), "raw output got no frames");
    assert!(raw.iter().all(|f| (f.width, f.height) == (320, 240)));
    assert!(!encoded.is_empty(), "encoded output got no packets");
    assert!(
        mux.iter().map(|c| c.data.len()).sum::<usize>() > 0,
        "mux bytes"
    );
    assert!(lossy > 0, "lossy subscriber got no frames");
    Ok(())
}

/// RTSP → relay (transcode) → RTSP: one bus pulls a live stream, re-encodes it
/// and publishes to another RTSP server, whose recording has the new geometry
/// and every frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn test_rtsp_relay_transcode() -> anyhow::Result<()> {
    if skip_without_test_mp4() {
        return Ok(());
    }
    let (ingest, egress) = (rtsp_url()?, rtsp_url()?);
    let path = test_media("rtsp_relay.mp4");
    std::fs::remove_file(&path).ok();

    // Final hop: records whatever the relay publishes.
    let sink = listening_bus(&egress, false).await?;
    let recording = spawn_first_output(&sink, file_output("file", &path));
    // Give the sink time to start listening before the relay dials it.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Relay: listens on `ingest`, re-encodes, publishes to `egress`.
    let relay = listening_bus(&ingest, false).await?;
    let relay_out = spawn_first_output(
        &relay,
        OutputConfig::new(
            "relay".to_string(),
            OutputAvType::Video,
            OutputDest::Net {
                url: egress.clone(),
                format: Some("rtsp".to_string()),
            },
        )
        .with_encode(h264(160, 120)),
    );

    let _tx = publish_file(&ingest).await?;
    let _ = join_output(relay_out).await?;
    let _ = join_output(recording).await?;

    wait_for_file(&path).await?;
    let v = video_summary(&path)?;
    assert_eq!((v.width, v.height), (160, 120));
    assert_eq!(v.frames, 50, "relay is lossless end to end");
    let duration = probe(&path)?.format.duration_sec.unwrap_or(0.0);
    assert!((4.5..=5.5).contains(&duration), "duration {duration}s");
    Ok(())
}

/// The publisher vanishing mid-stream ends every consumer (no hang) and the
/// recording is still finalized.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_rtsp_publisher_drop_ends_stream() -> anyhow::Result<()> {
    let url = rtsp_url()?;
    let path = test_media("rtsp_publisher_drop.mp4");
    std::fs::remove_file(&path).ok();

    let rx = listening_bus(&url, true).await?;
    let first = spawn_first_output(&rx, file_output("file", &path).with_encode(h264(160, 120)));
    let tx = publish_live(&url, 60).await?;
    let _ = join_output(first).await?;
    let decoded = spawn_count_decoded(rx.subscribe_video().await?);

    tokio::time::sleep(Duration::from_millis(1500)).await;
    drop(tx);

    let frames = finish(decoded, "decoded subscriber after publisher drop").await?;
    assert!(frames > 10, "frames before the drop: {frames}");
    wait_for_file(&path).await?;
    let v = video_summary(&path)?;
    assert!(v.frames > 10, "recorded frames: {}", v.frames);
    // GOP 25 at 25fps: ~1.5s of live video carries at least two keyframes.
    assert!(v.keys >= 2, "recorded keyframes: {}", v.keys);
    Ok(())
}

/// Removing the input of a still-running live stream tears every consumer
/// down promptly (no leaked relay waiting forever).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_rtsp_remove_input_while_live() -> anyhow::Result<()> {
    let url = rtsp_url()?;
    let rx = listening_bus(&url, true).await?;
    let first = spawn_first_output(
        &rx,
        OutputConfig::new(
            "demuxed".to_string(),
            OutputAvType::Video,
            OutputDest::Demuxed,
        ),
    );
    let _tx = publish_live(&url, 60).await?;
    let (_, demuxed) = join_output(first).await?;
    let demuxed = spawn_drain(demuxed);
    let decoded = spawn_count_decoded(rx.subscribe_video().await?);

    tokio::time::sleep(Duration::from_secs(1)).await;
    rx.remove_input().await?;

    let short = Duration::from_secs(5);
    let frames = tokio::time::timeout(short, decoded)
        .await
        .map_err(|_| anyhow::anyhow!("decoded subscriber hung after remove_input"))??;
    let packets = tokio::time::timeout(short, demuxed)
        .await
        .map_err(|_| anyhow::anyhow!("demuxed output hung after remove_input"))??;
    assert!(frames > 0 && !packets.is_empty());
    Ok(())
}

/// Smoke test of the live mux path: a File transcode far slower than real
/// time next to a demuxed passthrough on a live source. The passthrough keeps
/// real time and the slow recording is still finalized on teardown. Note: in
/// 3s the old lossless coupling would not show yet (it needs the 4096-packet
/// input buffer to fill, minutes of live video); the policy wiring itself is
/// checked by `bus_test::test_live_input_file_output_is_lossy`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_rtsp_live_slow_output_does_not_stall_others() -> anyhow::Result<()> {
    let url = rtsp_url()?;
    let path = test_media("rtsp_live_slow.mp4");
    std::fs::remove_file(&path).ok();

    let rx = listening_bus(&url, true).await?;
    let first = spawn_first_output(
        &rx,
        OutputConfig::new(
            "demuxed".to_string(),
            OutputAvType::Video,
            OutputDest::Demuxed,
        ),
    );
    let started = Instant::now();
    let _tx = publish_live(&url, 3).await?;
    let (_, demuxed) = join_output(first).await?;
    let demuxed = spawn_drain(demuxed);

    // Upscale to 720p with x264's slowest preset: far below 25fps.
    let slow = EncodeConfig {
        width: Some(1280),
        height: Some(720),
        preset: Some("placebo".to_string()),
        ..Default::default()
    };
    let _ = rx
        .add_output(file_output("slow", &path).with_encode(slow))
        .await?;

    let frames = finish(demuxed, "demuxed passthrough").await?;
    let elapsed = started.elapsed();
    assert!(frames.len() >= 70, "passthrough frames: {}", frames.len());
    assert!(
        elapsed < Duration::from_secs(8),
        "passthrough held back by the slow output: {elapsed:?} for 3s of video"
    );

    // Tear down: the slow encoder is cancelled and the recording finalized.
    drop(rx);
    wait_for_file(&path).await?;
    let v = video_summary(&path)?;
    assert_eq!((v.width, v.height), (1280, 720));
    assert!(
        v.frames > 0 && v.keys >= 1,
        "recording: {} frames",
        v.frames
    );
    Ok(())
}
