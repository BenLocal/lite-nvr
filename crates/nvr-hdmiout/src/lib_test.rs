use super::*;

const MODE: DisplayMode = DisplayMode {
    width: 2,
    height: 2,
    refresh_hz: 60,
};

#[test]
fn copies_pixels_with_independent_source_and_scanout_padding() {
    let source = [
        1, 2, 3, 0, 4, 5, 6, 0, 99, 99, 99, 99, 7, 8, 9, 0, 10, 11, 12, 0,
    ];
    let mut target = [42; 24];
    copy_frame(
        VideoFrame {
            width: 2,
            height: 2,
            stride: 12,
            data: &source,
        },
        MODE,
        &mut target,
        16,
    )
    .unwrap();
    assert_eq!(&target[..8], &source[..8]);
    assert_eq!(&target[8..16], &[42; 8]);
    assert_eq!(&target[16..], &source[12..]);
}

#[test]
fn rejects_bad_layouts_before_writing_any_pixels() {
    for (width, height, stride, length) in [
        (1, 2, 8, 16),
        (2, 0, 8, 16),
        (2, 2, 7, 16),
        (2, 2, 8, 15),
        (2, 2, usize::MAX, 16),
    ] {
        let data = vec![0; length];
        let mut target = [42; 16];
        assert!(matches!(
            copy_frame(
                VideoFrame {
                    width,
                    height,
                    stride,
                    data: &data
                },
                MODE,
                &mut target,
                8
            ),
            Err(Error::InvalidFrame)
        ));
        assert_eq!(target, [42; 16]);
    }
    let mut short = [42; 15];
    assert!(
        copy_frame(
            VideoFrame {
                width: 2,
                height: 2,
                stride: 8,
                data: &[0; 16]
            },
            MODE,
            &mut short,
            8
        )
        .is_err()
    );
    assert_eq!(short, [42; 15]);
}

#[test]
fn rejects_zero_width_and_overflow() {
    assert!(layout(0, 1, 4).is_err());
    assert!(layout(1, u32::MAX, usize::MAX).is_err());
}

#[cfg(not(target_os = "linux"))]
#[test]
fn unsupported_platform_reports_an_error() {
    assert!(matches!(
        HdmiOutput::open(&OutputConfig::default()),
        Err(Error::Unsupported)
    ));
    assert!(matches!(
        list_connectors("/dev/dri/card0"),
        Err(Error::Unsupported)
    ));
}
