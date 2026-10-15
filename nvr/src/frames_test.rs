use ffmpeg_bus::frame::RawVideoFrame;
use ffmpeg_next::format::Pixel;

#[test]
fn encode_jpeg_produces_a_decodable_picture() -> anyhow::Result<()> {
    let mut frame = ffmpeg_next::frame::Video::new(Pixel::YUV420P, 64, 48);
    for plane in 0..3 {
        frame.data_mut(plane).fill(128);
    }
    let jpeg = super::encode_jpeg(&RawVideoFrame::from(frame))?;
    assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "JPEG start-of-image marker");
    let decoded = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg)?;
    assert_eq!((decoded.width(), decoded.height()), (64, 48));
    Ok(())
}

#[tokio::test]
async fn snapshot_of_unknown_device_fails() {
    let err = super::snapshot_jpeg("no-such-device", std::time::Duration::from_millis(100))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no running pipe"), "{err:#}");
}
