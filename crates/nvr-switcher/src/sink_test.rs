use super::*;

#[test]
fn program_clock_and_output_parameters_are_independent_of_source() -> Result<()> {
    ffmpeg_bus::init()?;
    let input_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../e2e/test.mp4");
    let input = ffmpeg_next::format::input(&input_path)?;
    let source: AvStream = input
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .unwrap()
        .into();
    assert_ne!(
        source.time_base(),
        ffmpeg_next::util::mathematics::rescale::TIME_BASE
    );
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("program.ts");
    let mut sink = ProgramSink::new(
        &source,
        &ProgramSinkConfig {
            publish_url: path.to_str().unwrap().into(),
            format: "mpegts".into(),
            width: 320,
            height: 240,
            fps: 25,
            bitrate: Some(500_000),
        },
    )?;
    assert_eq!(
        sink.encoder.output_stream(0).rate(),
        ffmpeg_next::Rational(25, 1)
    );
    for _ in 0..50 {
        sink.push(Video::new(Pixel::YUV420P, 320, 240), false)?;
    }
    sink.finish();
    drop(sink);
    let mut output = ffmpeg_next::format::input(&path)?;
    let video: AvStream = output
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .unwrap()
        .into();
    assert_eq!((video.width(), video.height()), (320, 240));
    assert_eq!(video.parameters().id(), ffmpeg_next::codec::Id::H264);
    let timestamps: Vec<_> = output
        .packets()
        .filter(|(stream, _)| stream.index() == video.index())
        .filter_map(|(_, packet)| packet.pts())
        .collect();
    assert_eq!(timestamps.len(), 50);
    let span = (timestamps.last().unwrap() - timestamps.first().unwrap()) as f64
        * f64::from(video.time_base());
    assert!((span - 1.96).abs() < 0.05, "expected 1.96s, got {span}s");
    Ok(())
}
