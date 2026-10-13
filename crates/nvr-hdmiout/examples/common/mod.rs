use nvr_hdmiout::{AudioConfig, AudioOutput};
use std::{
    error::Error,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

type DemoResult<T> = Result<T, Box<dyn Error>>;

pub fn stop_flag() -> DemoResult<Arc<AtomicBool>> {
    let stop = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
        signal_hook::flag::register(signal, Arc::clone(&stop))?;
    }
    Ok(stop)
}

pub fn deadline(seconds: u64) -> DemoResult<Option<Instant>> {
    if seconds == 0 {
        return Ok(None);
    }
    Ok(Some(
        Instant::now()
            .checked_add(Duration::from_secs(seconds))
            .ok_or("duration overflow")?,
    ))
}

pub fn running(deadline: Option<Instant>, stop: &AtomicBool) -> bool {
    !stop.load(Ordering::Relaxed) && deadline.is_none_or(|deadline| Instant::now() < deadline)
}

fn tone(samples: &mut [i16], phase: &mut f64, frequency: f64) {
    let step = std::f64::consts::TAU * frequency / 48000.0;
    for frame in samples.as_chunks_mut::<2>().0 {
        let sample = (phase.sin() * 4096.0) as i16;
        frame.fill(sample);
        *phase = (*phase + step) % std::f64::consts::TAU;
    }
}

/// Cancels and joins every audio writer even if video presentation fails.
pub struct TonePlayback {
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<nvr_hdmiout::Result<()>>>,
}

impl TonePlayback {
    pub fn start(devices: &[(&str, f64)], stop: Arc<AtomicBool>) -> DemoResult<Self> {
        let mut playback = Self {
            stop,
            workers: Vec::new(),
        };
        if !devices.is_empty() {
            // Allow the HDMI receiver's clock to settle after the DRM modeset.
            std::thread::sleep(Duration::from_secs(1));
        }
        for &(device, frequency) in devices {
            if !frequency.is_finite() || !(0.0..24000.0).contains(&frequency) || frequency == 0.0 {
                return Err("tone frequency must be greater than zero and below 24000 Hz".into());
            }
            let mut output = AudioOutput::open(&AudioConfig {
                device: device.into(),
                ..Default::default()
            })?;
            eprintln!("Audio: {device}, 48000 Hz stereo, {frequency} Hz tone");
            let stop = Arc::clone(&playback.stop);
            playback.workers.push(std::thread::spawn(move || {
                let result = (|| {
                    let mut phase = 0.0;
                    let mut samples = [0i16; 960];
                    while !stop.load(Ordering::Relaxed) {
                        tone(&mut samples, &mut phase, frequency);
                        output.write(&samples)?;
                    }
                    output.close()
                })();
                if result.is_err() {
                    stop.store(true, Ordering::Relaxed);
                }
                result
            }));
        }
        Ok(playback)
    }

    pub fn finish(&mut self) -> DemoResult<()> {
        self.stop.store(true, Ordering::Relaxed);
        let mut result = Ok(());
        for worker in std::mem::take(&mut self.workers) {
            let outcome = match worker.join() {
                Ok(outcome) => outcome.map_err(|error| Box::new(error) as Box<dyn Error>),
                Err(_) => Err(std::io::Error::other("HDMI audio writer panicked").into()),
            };
            if result.is_ok() {
                result = outcome;
            }
        }
        result
    }
}

impl Drop for TonePlayback {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod mod_test;
