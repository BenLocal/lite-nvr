//! Two independent moving patterns on the same DRM card.
use nvr_hdmiout::{HdmiOutput, OutputConfig, VideoFrame};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let seconds = args
        .next()
        .ok_or("usage: dual_hdmiout <seconds> <card> <first-connector> <second-connector>")?
        .parse::<u64>()?;
    let card = args.next().ok_or("missing DRM card")?;
    let first_connector = args.next().ok_or("missing first connector")?;
    let second_connector = args.next().ok_or("missing second connector")?;
    if args.next().is_some() || first_connector == second_connector {
        return Err("provide exactly two distinct connectors".into());
    }
    let mut first = HdmiOutput::open(&OutputConfig {
        card: card.into(),
        connector: Some(first_connector),
        ..Default::default()
    })?;
    let mut second = first.open_sibling(&second_connector, None)?;
    let mut frames = Vec::new();
    for output in [&first, &second] {
        let mode = output.mode();
        let stride = usize::try_from(mode.width)?
            .checked_mul(4)
            .ok_or("frame overflow")?;
        let size = stride
            .checked_mul(usize::try_from(mode.height)?)
            .ok_or("frame overflow")?;
        frames.push((mode, stride, vec![0; size], vec![0; stride]));
        eprintln!("{}x{}@{}", mode.width, mode.height, mode.refresh_hz);
    }
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(seconds))
        .ok_or("duration overflow")?;
    let mut count = 0u64;
    while Instant::now() < deadline {
        for (index, output) in [&mut first, &mut second].into_iter().enumerate() {
            let (mode, stride, pixels, line) = &mut frames[index];
            for (x, pixel) in line.chunks_exact_mut(4).enumerate() {
                let marker =
                    (u64::try_from(x)? + count.wrapping_mul(8)) % u64::from(mode.width) < 80;
                let color = if marker {
                    [255, 255, 255, 0]
                } else if index == 0 {
                    [0, 0, 220, 0]
                } else {
                    [220, 0, 0, 0]
                };
                pixel.copy_from_slice(&color);
            }
            for row in pixels.chunks_exact_mut(*stride) {
                row.copy_from_slice(line);
            }
            output.present(VideoFrame {
                width: mode.width,
                height: mode.height,
                stride: *stride,
                data: pixels,
            })?;
        }
        count = count.wrapping_add(1);
    }
    first.close()?;
    // Closing the first connector must leave the second connector operational.
    let (mode, stride, pixels, _) = &frames[1];
    second.present(VideoFrame {
        width: mode.width,
        height: mode.height,
        stride: *stride,
        data: pixels,
    })?;
    eprintln!("Second output presented after first output closed");
    second.close()?;
    eprintln!("Presented {count} frames on each HDMI output");
    Ok(())
}
