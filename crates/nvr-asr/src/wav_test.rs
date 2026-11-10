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

#[test]
fn complete_wav_loader_rejects_wrong_sample_rate() {
    let path = std::env::temp_dir().join(format!("nvr-asr-rate-{}.wav", std::process::id()));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    writer.write_sample(0i16).unwrap();
    writer.finalize().unwrap();
    let result = super::load_wav_16k_mono(&path);
    std::fs::remove_file(path).unwrap();
    assert!(result.unwrap_err().to_string().contains("need 16000"));
}
