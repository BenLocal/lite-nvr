use super::*;

/// A fake decoder that turns each frame's channel-0 value into a constant
/// run of samples, so the stitched output shows which window wrote where.
fn fake_decode(window: &mut [f32]) -> Result<Vec<f32>> {
    let mut out = Vec::with_capacity(WINDOW * SAMPLES_PER_FRAME);
    for &v in &window[..WINDOW] {
        out.extend(std::iter::repeat_n(v, SAMPLES_PER_FRAME));
    }
    Ok(out)
}

fn latent(frames: usize) -> Vec<f32> {
    // Channel 0 holds the frame index; other channels are ignored.
    let mut l = vec![0.0; CHANNELS * frames];
    for (f, v) in l[..frames].iter_mut().enumerate() {
        *v = f as f32;
    }
    l
}

#[test]
fn short_sequences_take_one_padded_window() {
    let mut calls = 0;
    let audio = overlap_add(&latent(10), 10, |w| {
        calls += 1;
        fake_decode(w)
    })
    .unwrap();
    assert_eq!(calls, 1);
    assert_eq!(
        audio.len(),
        10 * SAMPLES_PER_FRAME,
        "trimmed to the real frames"
    );
    assert_eq!(audio[9 * SAMPLES_PER_FRAME], 9.0);
}

#[test]
fn windows_overlap_and_reproduce_a_consistent_signal() {
    let frames = 300;
    let mut calls = 0;
    let audio = overlap_add(&latent(frames), frames, |w| {
        calls += 1;
        fake_decode(w)
    })
    .unwrap();
    // Windows start at 0, 96, 192: the last one covers 192..320.
    assert_eq!(calls, 3);
    assert_eq!(audio.len(), frames * SAMPLES_PER_FRAME);
    // Every window sees the same latent for a frame, so cross-fading two
    // agreeing windows must give back the frame value everywhere.
    for f in 0..frames {
        let s = audio[f * SAMPLES_PER_FRAME + SAMPLES_PER_FRAME / 2];
        assert!((s - f as f32).abs() < 1e-3, "frame {f}: {s}");
    }
}

#[test]
fn cross_fade_blends_disagreeing_windows() {
    // Window k outputs the constant k + 1: the overlap must ramp between them.
    let mut k = 0.0;
    let audio = overlap_add(&latent(200), 200, |_| {
        k += 1.0;
        Ok(vec![k; WINDOW * SAMPLES_PER_FRAME])
    })
    .unwrap();
    let start = (WINDOW - OVERLAP) * SAMPLES_PER_FRAME;
    let fade = OVERLAP * SAMPLES_PER_FRAME;
    assert_eq!(audio[start - 1], 1.0);
    assert!((audio[start + fade / 2] - 1.5).abs() < 1e-3);
    assert_eq!(audio[start + fade], 2.0);
}

#[test]
fn wrong_decoder_output_is_an_error() {
    let err = overlap_add(&latent(10), 10, |_| Ok(vec![0.0; 5])).unwrap_err();
    assert!(err.to_string().contains("expected"), "{err:#}");
}
