use super::resample::Pcm16kMono;
use ffmpeg_next::ChannelLayout;
use ffmpeg_next::format::{Sample, sample::Type};
use ffmpeg_next::frame::Audio;

#[test]
fn resamples_48k_stereo_to_16k_mono_count() {
    ffmpeg_next::init().ok();
    // 4800 samples @ 48k stereo -> ~1600 samples @ 16k mono (+/- filter delay).
    let mut frame = Audio::new(Sample::F32(Type::Packed), 4800, ChannelLayout::STEREO);
    frame.set_rate(48_000);
    let mut r = Pcm16kMono::new();
    let out = r.push(&frame).expect("resample");
    assert!(
        (1400..=1700).contains(&out.len()),
        "expected ~1600 mono samples, got {}",
        out.len()
    );
}
