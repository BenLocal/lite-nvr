use super::*;

#[test]
fn stereo_tone_keeps_phase_between_buffers() {
    let mut whole = [0; 1920];
    tone(&mut whole, &mut 0.0, 440.0);
    let mut chunks = [0; 1920];
    let mut phase = 0.0;
    for block in chunks.chunks_mut(960) {
        tone(block, &mut phase, 440.0);
    }
    assert_eq!(chunks, whole);
    assert!(whole.chunks_exact(2).all(|frame| frame[0] == frame[1]));
    assert!(whole.iter().any(|sample| sample.abs() > 4000));
}

#[test]
fn continuous_output_obeys_stop_flag() {
    assert!(deadline(0).unwrap().is_none());
    let stop = AtomicBool::new(false);
    assert!(running(None, &stop));
    stop.store(true, Ordering::Relaxed);
    assert!(!running(None, &stop));
}
