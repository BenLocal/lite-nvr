use super::*;

#[test]
fn test_dynamic_mixer_new() -> anyhow::Result<()> {
    let _mixer = DynamicMixer::new(2, 48_000)?;
    Ok(())
}

#[test]
fn test_dynamic_mixer_push_silence_and_pull() -> anyhow::Result<()> {
    let mixer = DynamicMixer::new(2, 48_000)?;
    let (read, write) = mixer.split();
    let samples_per_channel = 1024_usize;
    let pts = 0_i64;

    write.push_silence(0, samples_per_channel, pts)?;
    write.push_silence(1, samples_per_channel, pts)?;

    let mut out_count = 0_usize;
    while let Some(frame) = read.pull_frame()? {
        out_count += 1;
        assert!(frame.samples() > 0, "mixed frame should carry samples");
        if out_count >= 10 {
            break;
        }
    }
    assert!(out_count >= 1, "expected at least one mixed output frame");
    Ok(())
}

#[test]
fn test_dynamic_mixer_push_audio_and_pull() -> anyhow::Result<()> {
    let mixer = DynamicMixer::new(2, 48_000)?;
    let (read, write) = mixer.split();
    let samples_per_channel = 512_usize;
    let make_silence_frame = || {
        let mut frame = Audio::new(
            Sample::I16(ffmpeg_next::format::sample::Type::Packed),
            samples_per_channel,
            ChannelLayout::STEREO,
        );
        frame.set_rate(48_000);
        frame.set_pts(Some(0));
        for plane in 0..frame.planes() {
            for b in frame.data_mut(plane) {
                *b = 0;
            }
        }
        frame
    };

    write.push_audio(0, &make_silence_frame())?;
    write.push_audio(1, &make_silence_frame())?;

    let mut out_count = 0_usize;
    while let Some(out_frame) = read.pull_frame()? {
        out_count += 1;
        assert!(out_frame.samples() > 0);
        if out_count >= 5 {
            break;
        }
    }
    assert!(out_count >= 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_dynamic_mixer_task() -> anyhow::Result<()> {
    let mixer = DynamicMixer::new(2, 48_000)?;
    let mut task = DynamicMixerTask::new();

    let mut out_rx = task.subscribe();
    task.start(mixer).await?;

    let (input0_tx, _) = tokio::sync::broadcast::channel::<RawFrameCmd>(32);
    let (input1_tx, _) = tokio::sync::broadcast::channel::<RawFrameCmd>(32);

    task.add_input(0, input0_tx.subscribe()).await?;
    task.add_input(1, input1_tx.subscribe()).await?;
    tokio::task::yield_now().await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let make_silence = || {
        let mut frame = Audio::new(
            Sample::I16(ffmpeg_next::format::sample::Type::Packed),
            512,
            ChannelLayout::STEREO,
        );
        frame.set_rate(48_000);
        frame.set_pts(Some(0));
        for plane in 0..frame.planes() {
            for b in frame.data_mut(plane) {
                *b = 0;
            }
        }
        RawFrameCmd::Data(RawFrame::Audio(frame.into()))
    };

    for _ in 0..8 {
        let _ = input0_tx.send(make_silence());
        let _ = input1_tx.send(make_silence());
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let mut out_count = 0_usize;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && out_count < 3 {
        match tokio::time::timeout(std::time::Duration::from_millis(500), out_rx.recv()).await {
            Ok(Ok(RawFrameCmd::Data(RawFrame::Audio(_)))) => out_count += 1,
            Ok(Ok(RawFrameCmd::Data(RawFrame::Video(_)))) => {}
            Ok(Ok(RawFrameCmd::EOF)) => break,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
            Err(_) => continue,
        }
    }

    task.cancel();
    assert!(
        out_count >= 1,
        "expected at least one mixed output frame, got out_count={}",
        out_count
    );
    Ok(())
}

#[tokio::test]
async fn test_dynamic_mixer_task_add_input_before_start_returns_error() {
    let task = DynamicMixerTask::new();
    let (_, rx) = tokio::sync::broadcast::channel::<RawFrameCmd>(8);
    let err = task.add_input(0, rx).await.err();
    assert!(err.is_some());
}

#[tokio::test]
async fn test_dynamic_mixer_task_remove_input_before_start_returns_error() {
    let task = DynamicMixerTask::new();
    let err = task.remove_input(0).await.err();
    assert!(err.is_some());
}

