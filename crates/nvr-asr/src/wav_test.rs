use super::downmix_to_mono;

#[test]
fn downmix_mono_passthrough() {
    let mono = [0.1, 0.2, 0.3];
    assert_eq!(downmix_to_mono(&mono, 1), vec![0.1, 0.2, 0.3]);
}

#[test]
fn downmix_stereo_averages() {
    // Interleaved L,R frames: (0,1)->0.5, (1,1)->1.0, (-1,1)->0.0.
    let stereo = [0.0, 1.0, 1.0, 1.0, -1.0, 1.0];
    assert_eq!(downmix_to_mono(&stereo, 2), vec![0.5, 1.0, 0.0]);
}

#[test]
fn downmix_zero_channels_treated_as_mono() {
    let s = [0.5, 0.5];
    assert_eq!(downmix_to_mono(&s, 0), vec![0.5, 0.5]);
}
