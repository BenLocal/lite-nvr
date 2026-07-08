use super::*;

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
    let frame = take_frame(&mut buf);
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
