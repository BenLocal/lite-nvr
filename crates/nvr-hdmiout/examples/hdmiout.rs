mod common;
use nvr_hdmiout::{HdmiOutput, OutputConfig, VideoFrame, list_connectors};
use std::{
    io::{self, Read},
    path::PathBuf,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("list");
    let config = OutputConfig {
        card: PathBuf::from(args.get(1).map(String::as_str).unwrap_or("/dev/dri/card0")),
        connector: args.get(2).cloned(),
        mode: None,
    };
    if command == "list" {
        for connector in list_connectors(&config.card)? {
            println!("{connector:?}");
        }
        return Ok(());
    }
    if command != "pattern" && command != "stdin" {
        return Err("usage: hdmiout [list|pattern|stdin] [card] [connector] [seconds=30] [alsa-device] [tone-hz=440]; seconds=0 runs continuously".into());
    }
    let mut output = HdmiOutput::open(&config)?;
    let mode = output.mode();
    eprintln!(
        "Output: {}x{} @ {} Hz; stdin format: bgr0",
        mode.width, mode.height, mode.refresh_hz
    );
    let stride = usize::try_from(mode.width)?
        .checked_mul(4)
        .ok_or("frame overflow")?;
    let length = stride
        .checked_mul(usize::try_from(mode.height)?)
        .ok_or("frame overflow")?;
    let mut pixels = vec![0; length];
    let seconds = args
        .get(3)
        .map(|v| v.parse::<u64>())
        .transpose()?
        .unwrap_or(30);
    let stop = if command == "pattern" {
        common::stop_flag()?
    } else {
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false))
    };
    let deadline = common::deadline(seconds)?;
    let frequency = args
        .get(5)
        .map(|value| value.parse::<f64>())
        .transpose()?
        .unwrap_or(440.0);
    let devices = args
        .get(4)
        .map(|device| vec![(device.as_str(), frequency)])
        .unwrap_or_default();
    let mut audio = common::TonePlayback::start(&devices, std::sync::Arc::clone(&stop))?;
    let mut input = io::stdin().lock();
    let colors = [
        [255, 255, 255, 0],
        [0, 255, 255, 0],
        [255, 255, 0, 0],
        [0, 255, 0, 0],
        [255, 0, 255, 0],
        [0, 0, 255, 0],
        [255, 0, 0, 0],
        [0, 0, 0, 0],
    ];
    let width = usize::try_from(mode.width)?;
    let half_height = usize::try_from(mode.height)? / 2;
    let mut line = vec![0; stride];
    if command == "pattern" {
        for (x, pixel) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            pixel.copy_from_slice(&colors[x * colors.len() / width]);
        }
        for row in pixels.chunks_exact_mut(stride).take(half_height) {
            row.copy_from_slice(&line);
        }
    }
    let mut count = 0u64;
    while common::running(deadline, &stop) {
        if command == "stdin" {
            match input.read_exact(&mut pixels) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => break,
                Err(error) => return Err(error.into()),
            }
        } else {
            // Generate a single moving row, then copy it; avoid rendering every
            // pixel with integer division on each frame of the hardware demo.
            for (x, pixel) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let bright =
                    (u64::try_from(x)? + count.wrapping_mul(8)) % u64::from(mode.width) < 64;
                pixel.copy_from_slice(if bright { &[255, 255, 255, 0] } else { &[0; 4] });
            }
            for row in pixels.chunks_exact_mut(stride).skip(half_height) {
                row.copy_from_slice(&line);
            }
        }
        output.present(VideoFrame {
            width: mode.width,
            height: mode.height,
            stride,
            data: &pixels,
        })?;
        count = count.wrapping_add(1);
    }
    audio.finish()?;
    output.close()?;
    eprintln!("Presented {count} frames");
    Ok(())
}
