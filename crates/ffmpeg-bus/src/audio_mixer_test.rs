use super::*;

async fn next_audio(receiver: &mut RawFrameReceiver) -> Audio {
    let frame = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    let RawFrameCmd::Data(RawFrame::Audio(frame)) = frame else {
        panic!("expected mixed PCM")
    };
    frame.as_audio().clone()
}

fn assert_pcm(frame: &Audio, expected: i16) {
    assert!(
        frame.data(0)[..frame.samples() * CHANNELS * 2]
            .as_chunks::<2>()
            .0
            .iter()
            .all(|bytes| i16::from_ne_bytes(*bytes) == expected)
    );
}

#[tokio::test]
async fn regression_ended_input_drains_buffered_tail() {
    for explicit_eof in [true, false] {
        let mixer = DynamicMixerTask::new(48_000);
        let (input_tx, input) = tokio::sync::broadcast::channel(8);
        mixer.add_input("clip", input, DEFAULT_VOLUME);
        let mut output = mixer.subscribe();
        for (index, value) in [1000, 2000, 3000].into_iter().enumerate() {
            input_tx
                .send(RawFrameCmd::Data(RawFrame::Audio(
                    build_frame(
                        &vec![value; FRAME_LEN],
                        48_000,
                        (index * FRAME_SAMPLES) as i64,
                    )
                    .into(),
                )))
                .ok()
                .unwrap();
        }
        if explicit_eof {
            input_tx.send(RawFrameCmd::EOF).ok().unwrap();
        }
        drop(input_tx);
        mixer.start();
        let _stop = mixer.cancel.clone().drop_guard();
        let mut frames = Vec::new();
        for _ in 0..4 {
            frames.push(next_audio(&mut output).await);
        }
        mixer.stop_and_join().await;
        for (frame, value) in frames.iter().zip([1000, 2000, 3000, 0]) {
            assert_pcm(frame, value);
        }
    }
}

#[tokio::test]
async fn regression_program_backlog_preserves_order_and_original_alignment() {
    let (program_tx, program) = tokio::sync::broadcast::channel(8);
    let mixer = DynamicMixerTask::with_program(48_000, program);
    let (original_tx, original) = tokio::sync::broadcast::channel(8);
    mixer.add_input("original", original, DEFAULT_VOLUME);
    let mut output = mixer.subscribe();
    for (index, value) in [1000, 2000, 3000].into_iter().enumerate() {
        program_tx
            .send(ProgramPcmFrame {
                samples: vec![i32::from(value); FRAME_LEN].into(),
                pts: 96_000 + (index * FRAME_SAMPLES) as i64,
                sample_rate: 48_000,
            })
            .ok()
            .unwrap();
        original_tx
            .send(RawFrameCmd::Data(RawFrame::Audio(
                build_frame(
                    &vec![value / 10; FRAME_LEN],
                    48_000,
                    (index * FRAME_SAMPLES) as i64,
                )
                .into(),
            )))
            .ok()
            .unwrap();
    }
    mixer.start();
    let _stop = mixer.cancel.clone().drop_guard();
    let first = next_audio(&mut output).await;
    // Check before receiving the rest: the old loop only emits the last frame.
    if first.pts() != Some(96_000) {
        mixer.stop_and_join().await;
        panic!("program backlog skipped the first frame: {:?}", first.pts());
    }
    let second = next_audio(&mut output).await;
    let third = next_audio(&mut output).await;
    mixer.stop_and_join().await;
    for (index, (frame, value)) in [first, second, third]
        .iter()
        .zip([1100, 2200, 3300])
        .enumerate()
    {
        assert_eq!(frame.pts(), Some(96_000 + (index * FRAME_SAMPLES) as i64));
        assert_pcm(frame, value);
    }
}

#[tokio::test]
async fn regression_ended_input_flushes_resampler_tail() {
    let mixer = DynamicMixerTask::new(48_000);
    let (input_tx, input) = tokio::sync::broadcast::channel(4);
    mixer.add_input("clip", input, DEFAULT_VOLUME);
    mixer.set_channel_map("clip", 1).unwrap();
    let mut output = mixer.subscribe();
    let mut frame = Audio::new(OUT_FMT, 4410, ChannelLayout::STEREO);
    frame.set_rate(44_100);
    for pair in frame.plane_mut::<(i16, i16)>(0) {
        *pair = (1000, -2000);
    }
    assert!(
        input_tx
            .send(RawFrameCmd::Data(RawFrame::Audio(frame.into())))
            .is_ok()
    );
    assert!(input_tx.send(RawFrameCmd::EOF).is_ok());
    mixer.start();
    let _stop = mixer.cancel.clone().drop_guard();
    let mut samples = Vec::new();
    for _ in 0..5 {
        let frame = next_audio(&mut output).await;
        samples.extend_from_slice(frame.plane::<(i16, i16)>(0));
    }
    mixer.stop_and_join().await;
    assert_eq!(samples.len(), 5120);
    assert!(
        samples[..4800]
            .iter()
            .all(|&(l, r)| (l + 2000).abs() <= 1 && (r - 1000).abs() <= 1)
    );
    assert!(samples[4800..].iter().all(|&pair| pair == (0, 0)));
}

#[tokio::test]
async fn regression_program_overrun_skips_matching_original_samples() {
    let (program_tx, program) = tokio::sync::broadcast::channel(2);
    let mixer = DynamicMixerTask::with_program(48_000, program);
    let (original_tx, original) = tokio::sync::broadcast::channel(8);
    mixer.add_input("original", original, DEFAULT_VOLUME);
    let mut output = mixer.subscribe();
    for index in 0..4 {
        assert!(
            program_tx
                .send(ProgramPcmFrame {
                    samples: vec![1000; FRAME_LEN].into(),
                    pts: (index * FRAME_SAMPLES) as i64,
                    sample_rate: 48_000,
                })
                .is_ok()
        );
        assert!(
            original_tx
                .send(RawFrameCmd::Data(RawFrame::Audio(
                    build_frame(
                        &vec![(index + 1) as i16 * 100; FRAME_LEN],
                        48_000,
                        (index * FRAME_SAMPLES) as i64
                    )
                    .into(),
                )))
                .is_ok()
        );
    }
    mixer.start();
    let _stop = mixer.cancel.clone().drop_guard();
    let first = next_audio(&mut output).await;
    if first.pts() != Some((2 * FRAME_SAMPLES) as i64) {
        mixer.stop_and_join().await;
        panic!(
            "program overrun skipped a retained frame: {:?}",
            first.pts()
        );
    }
    let second = next_audio(&mut output).await;
    mixer.stop_and_join().await;
    assert_pcm(&first, 1300);
    assert_pcm(&second, 1400);
    assert_eq!(second.pts(), Some((3 * FRAME_SAMPLES) as i64));
}

#[tokio::test]
async fn regression_original_buffer_overflow_does_not_mix_future_audio() {
    for capacity in [64, 2] {
        let (program_tx, program) = tokio::sync::broadcast::channel(capacity);
        let mixer = DynamicMixerTask::with_program(48_000, program);
        let (original_tx, original) = tokio::sync::broadcast::channel(64);
        mixer.add_input("original", original, DEFAULT_VOLUME);
        let mut output = mixer.subscribe();
        const FRAMES: usize = 50;
        for index in 0..FRAMES {
            assert!(
                program_tx
                    .send(ProgramPcmFrame {
                        samples: vec![1000; FRAME_LEN].into(),
                        pts: (index * FRAME_SAMPLES) as i64,
                        sample_rate: 48_000,
                    })
                    .is_ok()
            );
            assert!(
                original_tx
                    .send(RawFrameCmd::Data(RawFrame::Audio(
                        build_frame(
                            &vec![(index + 1) as i16 * 100; FRAME_LEN],
                            48_000,
                            (index * FRAME_SAMPLES) as i64
                        )
                        .into(),
                    )))
                    .is_ok()
            );
        }
        mixer.start();
        let _stop = mixer.cancel.clone().drop_guard();
        let mut frames = Vec::new();
        for _ in 0..FRAMES.min(capacity) {
            frames.push(next_audio(&mut output).await);
        }
        mixer.stop_and_join().await;
        let missing = FRAMES * FRAME_SAMPLES - 48_000;
        for (offset, frame) in frames.iter().enumerate() {
            let index = FRAMES - FRAMES.min(capacity) + offset;
            assert_eq!(frame.pts(), Some((index * FRAME_SAMPLES) as i64));
            for (sample, &(l, r)) in frame.plane::<(i16, i16)>(0).iter().enumerate() {
                let expected = if index * FRAME_SAMPLES + sample < missing {
                    1000
                } else {
                    1000 + (index + 1) as i16 * 100
                };
                assert_eq!((l, r), (expected, expected));
            }
        }
    }
}

#[test]
fn resamples_pcm_with_unspecified_channel_order() {
    crate::init().unwrap();
    let mut input = Audio::new(OUT_FMT, FRAME_SAMPLES, ChannelLayout::STEREO);
    input.set_rate(48_000);
    for sample in input.plane_mut::<(i16, i16)>(0) {
        *sample = (1000, -2000);
    }
    let mut layout = ChannelLayout::STEREO;
    layout.0.order = ffmpeg_next::ffi::AVChannelOrder::AV_CHANNEL_ORDER_UNSPEC;
    input.set_channel_layout(layout);
    let samples = SlotResampler::new(48_000).convert(&input).unwrap();
    assert_eq!(samples.len(), FRAME_LEN);
    assert!(
        samples
            .as_chunks::<2>()
            .0
            .iter()
            .all(|pair| pair == &[1000, -2000])
    );
}

// ---- pure PCM helpers -----------------------------------------------------

#[test]
fn gain_unity_mute_and_scale() {
    assert_eq!(gain_factor(100, false), 1.0);
    assert_eq!(gain_factor(0, false), 0.0);
    assert_eq!(gain_factor(100, true), 0.0);
    assert_eq!(gain_factor(50, false), 0.5);
    // Runaway volume is capped at 4x.
    assert_eq!(gain_factor(10_000, false), 4.0);
}

#[test]
fn accumulate_sums_multiple_inputs_with_gain() {
    let mut acc = vec![0i32; 4];
    accumulate(&mut acc, &[100, -100, 100, -100], 1.0);
    accumulate(&mut acc, &[50, 50, 50, 50], 1.0);
    assert_eq!(acc, vec![150, -50, 150, -50]);

    let mut acc = vec![0i32; 2];
    accumulate(&mut acc, &[100, 200], 0.5);
    assert_eq!(acc, vec![50, 100]);
}

#[test]
fn clamp_saturates_instead_of_wrapping() {
    let acc = vec![40_000, -40_000, 100, i16::MAX as i32];
    assert_eq!(clamp_to_i16(&acc), vec![i16::MAX, i16::MIN, 100, i16::MAX]);

    // Two near-full-scale inputs must clip to the rail, never wrap negative.
    let mut acc = vec![0i32; 2];
    accumulate(&mut acc, &[30_000, 30_000], 1.0);
    accumulate(&mut acc, &[30_000, 30_000], 1.0);
    assert_eq!(clamp_to_i16(&acc), vec![i16::MAX, i16::MAX]);
}

#[test]
fn take_frame_pads_short_buffer_with_silence() {
    let mut buf: VecDeque<i16> = VecDeque::from(vec![1, 2, 3]);
    let frame = take_frame(&mut buf, 0);
    assert_eq!(frame.len(), FRAME_LEN);
    assert_eq!(&frame[..3], &[1, 2, 3]);
    assert!(frame[3..].iter().all(|&s| s == 0));
    assert!(buf.is_empty());
}

// ---- task control surface (no running loop required) ----------------------

#[test]
fn add_set_and_remove_inputs_tracks_controls() {
    let task = DynamicMixerTask::new(48_000);
    let (_tx_a, rx_a) = tokio::sync::broadcast::channel::<RawFrameCmd>(8);
    let (_tx_b, rx_b) = tokio::sync::broadcast::channel::<RawFrameCmd>(8);

    task.add_input("a", rx_a, DEFAULT_VOLUME);
    task.add_input("b", rx_b, 40);

    let mut inputs = task.inputs();
    inputs.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        inputs,
        vec![("a".to_string(), 100, false), ("b".to_string(), 40, false),]
    );

    task.set_volume("a", 25).unwrap();
    task.set_muted("b", true).unwrap();
    let mut inputs = task.inputs();
    inputs.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(
        inputs,
        vec![("a".to_string(), 25, false), ("b".to_string(), 40, true),]
    );

    task.remove_input("a").unwrap();
    assert_eq!(task.inputs(), vec![("b".to_string(), 40, true)]);
}

#[test]
fn control_of_missing_input_errors() {
    let task = DynamicMixerTask::new(48_000);
    assert!(task.remove_input("nope").is_err());
    assert!(task.set_volume("nope", 50).is_err());
    assert!(task.set_muted("nope", true).is_err());
}

#[tokio::test]
async fn device_original_is_added_before_the_program_is_clipped() {
    let (program_tx, program) = tokio::sync::broadcast::channel(4);
    let mixer = DynamicMixerTask::with_program(48_000, program);
    let (original_tx, original) = tokio::sync::broadcast::channel(4);
    mixer.add_input("original", original, 100);
    let mut output = mixer.subscribe();
    mixer.start();
    assert!(
        original_tx
            .send(RawFrameCmd::Data(RawFrame::Audio(
                build_frame(&vec![-30_000; FRAME_LEN], 48_000, 0).into()
            )))
            .is_ok()
    );
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        program_tx
            .send(ProgramPcmFrame {
                samples: vec![45_000; FRAME_LEN].into(),
                pts: 96_000,
                sample_rate: 48_000
            })
            .is_ok()
    );
    let frame = tokio::time::timeout(Duration::from_secs(2), output.recv())
        .await
        .unwrap()
        .unwrap();
    mixer.stop_and_join().await;
    let RawFrameCmd::Data(RawFrame::Audio(frame)) = frame else {
        panic!("expected mixed PCM")
    };
    assert_eq!(frame.pts(), Some(96_000));
    for sample in frame.as_audio().data(0)[..FRAME_LEN * 2].as_chunks::<2>().0 {
        assert_eq!(i16::from_ne_bytes(*sample), 15_000);
    }
}

#[test]
fn input_mapping_changes_live_controls_and_validates_channel_choices() {
    let mixer = DynamicMixerTask::new(48_000);
    let (_tx, rx) = tokio::sync::broadcast::channel(4);
    mixer.add_input("stereo", rx, 100);
    assert_eq!(mixer.channel_map("stereo"), 0);
    mixer.set_channel_map("stereo", 1).unwrap();
    assert_eq!(mixer.channel_map("stereo"), 1);
    assert!(mixer.set_channel_map("stereo", 4).is_err());
    assert!(mixer.set_channel_map("missing", 0).is_err());
}
