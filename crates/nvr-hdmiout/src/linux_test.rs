use super::*;

#[test]
fn hardware_present_and_close() {
    let Some(card) = std::env::var_os("NVR_HDMI_TEST_CARD") else {
        return;
    };
    let config = OutputConfig {
        card: card.into(),
        connector: std::env::var("NVR_HDMI_TEST_CONNECTOR").ok(),
        mode: None,
    };
    for _ in 0..2 {
        // The loopback receiver briefly drops HPD after the previous modeset
        // is disabled. Wait for its connection/EDID to settle before reopening.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let connected = list_connectors(&config.card).unwrap().iter().any(|c| {
                c.state == ConnectionState::Connected
                    && !c.modes.is_empty()
                    && config.connector.as_ref().is_none_or(|name| *name == c.name)
            });
            if connected {
                break;
            }
            assert!(Instant::now() < deadline, "HDMI sink did not reconnect");
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut output = HdmiOutput::open(&config).unwrap();
        let mode = output.mode();
        let stride = usize::try_from(mode.width).unwrap() * 4;
        let pixels = vec![0; stride * usize::try_from(mode.height).unwrap()];
        assert!(matches!(
            output.present(VideoFrame {
                width: mode.width,
                height: mode.height,
                stride: stride - 1,
                data: &pixels
            }),
            Err(Error::InvalidFrame)
        ));
        for _ in 0..3 {
            output
                .present(VideoFrame {
                    width: mode.width,
                    height: mode.height,
                    stride,
                    data: &pixels,
                })
                .unwrap();
        }
        let mut sibling = std::env::var("NVR_HDMI_TEST_SIBLING").ok().map(|name| {
            assert!(matches!(
                output.open_sibling(config.connector.as_deref().unwrap_or("HDMI-A-1"), None),
                Err(Error::NoCrtc)
            ));
            output.open_sibling(&name, None).unwrap()
        });
        if let Some(sibling) = sibling.as_mut() {
            present_black(sibling);
        }
        output.close().unwrap();
        output.close().unwrap();
        if let Some(sibling) = sibling.as_mut() {
            // Releasing one connector must preserve the other connector's DRM master.
            present_black(sibling);
            sibling.close().unwrap();
            sibling.close().unwrap();
        }
        assert!(matches!(
            output.present(VideoFrame {
                width: mode.width,
                height: mode.height,
                stride,
                data: &pixels
            }),
            Err(Error::Closed)
        ));
    }
}

fn present_black(output: &mut HdmiOutput) {
    let mode = output.mode();
    let stride = usize::try_from(mode.width).unwrap() * 4;
    let pixels = vec![0; stride * usize::try_from(mode.height).unwrap()];
    output
        .present(VideoFrame {
            width: mode.width,
            height: mode.height,
            stride,
            data: &pixels,
        })
        .unwrap();
}
