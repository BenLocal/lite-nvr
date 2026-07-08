//! Dynamic audio mixing console primitive.
//!
//! [`DynamicMixerTask`] mixes any number of live audio inputs into ONE output
//! stream, with inputs added/removed and their volume/mute changed on the fly —
//! all without interrupting the output. It is the engine behind `nvr-audio-mixer`.
//!
//! Design (why not a fixed `amix` graph): a fixed-slot `amix` stalls until every
//! declared input is fed, attenuates by input count (`normalize=1`), and has no
//! per-input gain. Instead each input is resampled to a common interleaved-s16
//! stereo format and mixed by straight PCM summation (per-input gain, then a
//! single saturating clamp). That gives: a variable number of inputs, silence
//! for inputs that momentarily have no data, unity/`normalize=0` behaviour, and
//! live per-input volume/mute — exactly a mixing console.
//!
//! Threading: the mix runs on one dedicated blocking thread that owns all the
//! non-`Send` ffmpeg objects (per-input resamplers). Only `Send` values cross
//! into it — the broadcast `Receiver` for each input and the shared gain/mute
//! atomics — over a command channel. Volume/mute are plain atomics the loop
//! reads each tick, so they change with zero interruption.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ffmpeg_next::ChannelLayout;
use ffmpeg_next::format::{Sample, sample::Type};
use ffmpeg_next::frame::Audio;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

use crate::frame::{RawFrame, RawFrameCmd, RawFrameReceiver, RawFrameSender};

/// Channel count of the mix (stereo, interleaved).
const CHANNELS: usize = 2;
/// Samples *per channel* produced each mix tick (~21 ms at 48 kHz; AAC-friendly).
const FRAME_SAMPLES: usize = 1024;
/// Interleaved i16 values in one tick (`FRAME_SAMPLES * CHANNELS`).
const FRAME_LEN: usize = FRAME_SAMPLES * CHANNELS;
/// Output (mixed-frame) broadcast capacity.
const OUT_CHAN_CAP: usize = 128;
/// Default per-input volume (unity gain, percent).
pub const DEFAULT_VOLUME: u32 = 100;
/// Mixer output sample format: packed (interleaved) signed 16-bit.
const OUT_FMT: Sample = Sample::I16(Type::Packed);

// ---- pure PCM helpers (unit tested) ---------------------------------------

/// Turn a UI volume percentage + mute flag into a linear gain factor. `100` is
/// unity, `0` is silent, values above boost (capped at 4x / +12 dB). Muted is
/// always `0.0`.
pub(crate) fn gain_factor(volume_percent: u32, muted: bool) -> f32 {
    if muted {
        return 0.0;
    }
    (volume_percent as f32 / 100.0).clamp(0.0, 4.0)
}

/// Add one input's interleaved samples into a wide accumulator, scaled by
/// `gain`. Summing in `i32` avoids intermediate clipping; the clamp happens once
/// in [`clamp_to_i16`].
pub(crate) fn accumulate(acc: &mut [i32], samples: &[i16], gain: f32) {
    for (a, &s) in acc.iter_mut().zip(samples) {
        *a += (s as f32 * gain) as i32;
    }
}

/// Hard-limit a wide accumulator back to interleaved i16, saturating overflow.
pub(crate) fn clamp_to_i16(acc: &[i32]) -> Vec<i16> {
    acc.iter()
        .map(|&v| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16)
        .collect()
}

// ---- per-input resampler --------------------------------------------------

/// Resamples an arbitrary decoded audio frame to interleaved s16 stereo at the
/// mixer rate. The `swr` context is rebuilt only when the input format changes.
struct SlotResampler {
    rate: u32,
    swr: Option<(ffmpeg_next::software::resampling::Context, Sample, u32, u16)>,
}

impl SlotResampler {
    fn new(rate: u32) -> Self {
        Self { rate, swr: None }
    }

    fn convert(&mut self, input: &Audio) -> anyhow::Result<Vec<i16>> {
        let in_fmt = input.format();
        let in_rate = input.rate();
        let in_ch = input.channels();
        let stale = match &self.swr {
            Some((_, fmt, rate, ch)) => *fmt != in_fmt || *rate != in_rate || *ch != in_ch,
            None => true,
        };
        if stale {
            let ctx = ffmpeg_next::software::resampling::Context::get(
                in_fmt,
                input.channel_layout(),
                in_rate,
                OUT_FMT,
                ChannelLayout::STEREO,
                self.rate,
            )?;
            self.swr = Some((ctx, in_fmt, in_rate, in_ch));
        }
        let swr = &mut self.swr.as_mut().expect("swr set above").0;
        let max_out = unsafe {
            ffmpeg_next::ffi::swr_get_out_samples(swr.as_mut_ptr(), input.samples() as i32)
        };
        if max_out <= 0 {
            return Ok(Vec::new());
        }
        let mut out = Audio::new(OUT_FMT, max_out as usize, ChannelLayout::STEREO);
        swr.run(input, &mut out)?;
        let count = out.samples() * CHANNELS;
        let bytes = out.data(0);
        let mut samples = Vec::with_capacity(count);
        for i in 0..count {
            samples.push(i16::from_ne_bytes([bytes[i * 2], bytes[i * 2 + 1]]));
        }
        Ok(samples)
    }
}

// ---- public task ----------------------------------------------------------

/// Shared, live-tunable control for one input. The same `Arc`s are held by the
/// public API (writers) and the mix thread (reader).
struct InputControl {
    volume: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
}

/// Command into the mix thread. Carries only `Send` payloads.
enum MixerCmd {
    Add {
        id: String,
        receiver: RawFrameReceiver,
        volume: Arc<AtomicU32>,
        muted: Arc<AtomicBool>,
    },
    Remove {
        id: String,
    },
}

/// A running dynamic mixer. Inputs are keyed by an arbitrary string id.
pub struct DynamicMixerTask {
    sample_rate: u32,
    cancel: CancellationToken,
    out_tx: RawFrameSender,
    controls: Arc<Mutex<HashMap<String, InputControl>>>,
    cmd_tx: UnboundedSender<MixerCmd>,
    cmd_rx: Mutex<Option<UnboundedReceiver<MixerCmd>>>,
}

impl DynamicMixerTask {
    /// Create a mixer that produces stereo output at `sample_rate`. Call
    /// [`start`](Self::start) to begin mixing.
    pub fn new(sample_rate: u32) -> Self {
        let (cmd_tx, cmd_rx) = unbounded_channel();
        let (out_tx, _) = tokio::sync::broadcast::channel(OUT_CHAN_CAP);
        Self {
            sample_rate,
            cancel: CancellationToken::new(),
            out_tx,
            controls: Arc::new(Mutex::new(HashMap::new())),
            cmd_tx,
            cmd_rx: Mutex::new(Some(cmd_rx)),
        }
    }

    /// Subscribe to the mixed output (interleaved-s16 stereo `RawFrame::Audio`).
    pub fn subscribe(&self) -> RawFrameReceiver {
        self.out_tx.subscribe()
    }

    /// Start the mix loop on a dedicated blocking thread. Call once.
    pub fn start(&self) {
        let rx = self
            .cmd_rx
            .lock()
            .unwrap()
            .take()
            .expect("DynamicMixerTask::start called twice");
        let rate = self.sample_rate;
        let cancel = self.cancel.clone();
        let out = self.out_tx.clone();
        tokio::task::spawn_blocking(move || mix_loop(rate, cancel, rx, out));
    }

    /// Add (or replace) an input at the given volume (percent).
    pub fn add_input(&self, id: &str, receiver: RawFrameReceiver, volume: u32) {
        let volume = Arc::new(AtomicU32::new(volume));
        let muted = Arc::new(AtomicBool::new(false));
        self.controls.lock().unwrap().insert(
            id.to_string(),
            InputControl {
                volume: volume.clone(),
                muted: muted.clone(),
            },
        );
        let _ = self.cmd_tx.send(MixerCmd::Add {
            id: id.to_string(),
            receiver,
            volume,
            muted,
        });
    }

    /// Remove an input. Errors if it isn't on the mixer.
    pub fn remove_input(&self, id: &str) -> anyhow::Result<()> {
        if self.controls.lock().unwrap().remove(id).is_none() {
            anyhow::bail!("mixer input '{id}' not found");
        }
        let _ = self.cmd_tx.send(MixerCmd::Remove { id: id.to_string() });
        Ok(())
    }

    /// Set an input's volume (percent). Takes effect on the next tick.
    pub fn set_volume(&self, id: &str, volume: u32) -> anyhow::Result<()> {
        let controls = self.controls.lock().unwrap();
        let ctl = controls
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("mixer input '{id}' not found"))?;
        ctl.volume.store(volume, Ordering::Relaxed);
        Ok(())
    }

    /// Mute/unmute an input. Takes effect on the next tick.
    pub fn set_muted(&self, id: &str, muted: bool) -> anyhow::Result<()> {
        let controls = self.controls.lock().unwrap();
        let ctl = controls
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("mixer input '{id}' not found"))?;
        ctl.muted.store(muted, Ordering::Relaxed);
        Ok(())
    }

    /// Current inputs as `(id, volume, muted)`.
    pub fn inputs(&self) -> Vec<(String, u32, bool)> {
        self.controls
            .lock()
            .unwrap()
            .iter()
            .map(|(id, c)| {
                (
                    id.clone(),
                    c.volume.load(Ordering::Relaxed),
                    c.muted.load(Ordering::Relaxed),
                )
            })
            .collect()
    }

    /// Stop the mix loop.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

/// One input as seen by the mix thread.
struct Active {
    receiver: RawFrameReceiver,
    resampler: SlotResampler,
    buffer: VecDeque<i16>,
    volume: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
}

/// The mix loop: resample every input to the common format, sum the current
/// tick (gain-scaled, silence-padded), clamp, and broadcast one frame — paced to
/// real time. Runs until `cancel` fires.
fn mix_loop(
    rate: u32,
    cancel: CancellationToken,
    mut cmd_rx: UnboundedReceiver<MixerCmd>,
    out: RawFrameSender,
) {
    let max_buffer = rate as usize * CHANNELS; // ~1 s per input
    let tick = Duration::from_micros(FRAME_SAMPLES as u64 * 1_000_000 / rate.max(1) as u64);
    let mut inputs: HashMap<String, Active> = HashMap::new();
    let mut pts: i64 = 0;
    let mut next = Instant::now();

    while !cancel.is_cancelled() {
        // 1) Apply add/remove commands.
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                MixerCmd::Add {
                    id,
                    receiver,
                    volume,
                    muted,
                } => {
                    inputs.insert(
                        id,
                        Active {
                            receiver,
                            resampler: SlotResampler::new(rate),
                            buffer: VecDeque::new(),
                            volume,
                            muted,
                        },
                    );
                }
                MixerCmd::Remove { id } => {
                    inputs.remove(&id);
                }
            }
        }

        // 2) Drain each input's available frames into its jitter buffer.
        let mut dead: Vec<String> = Vec::new();
        for (id, active) in inputs.iter_mut() {
            loop {
                match active.receiver.try_recv() {
                    Ok(RawFrameCmd::Data(RawFrame::Audio(frame))) => {
                        match active.resampler.convert(frame.as_audio()) {
                            Ok(samples) => active.buffer.extend(samples),
                            Err(e) => log::warn!("mixer resample '{id}': {e:#}"),
                        }
                        if active.buffer.len() > max_buffer {
                            let overflow = active.buffer.len() - max_buffer;
                            active.buffer.drain(..overflow);
                        }
                    }
                    Ok(RawFrameCmd::Data(_)) => {}
                    Ok(RawFrameCmd::EOF) => {
                        dead.push(id.clone());
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Lagged(n)) => {
                        log::warn!("mixer input '{id}' lagged, dropped {n}");
                    }
                    Err(TryRecvError::Closed) => {
                        dead.push(id.clone());
                        break;
                    }
                }
            }
        }
        for id in dead {
            inputs.remove(&id);
        }

        // 3) Mix one tick: gain-scaled sum, then a single saturating clamp.
        let mut acc = vec![0i32; FRAME_LEN];
        for active in inputs.values_mut() {
            let gain = gain_factor(
                active.volume.load(Ordering::Relaxed),
                active.muted.load(Ordering::Relaxed),
            );
            let frame = take_frame(&mut active.buffer);
            if gain > 0.0 {
                accumulate(&mut acc, &frame, gain);
            }
        }
        let mixed = clamp_to_i16(&acc);

        // 4) Emit the mixed frame (dropped harmlessly if nobody is subscribed).
        let frame = build_frame(&mixed, rate, pts);
        pts += FRAME_SAMPLES as i64;
        let _ = out.send(RawFrameCmd::Data(RawFrame::Audio(frame.into())));

        // 5) Pace to real time.
        next += tick;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
    log::info!("audio mixer loop stopped");
}

/// Pop one tick of interleaved samples, silence-padded to exactly `FRAME_LEN`.
fn take_frame(buffer: &mut VecDeque<i16>) -> Vec<i16> {
    let mut out = Vec::with_capacity(FRAME_LEN);
    for _ in 0..FRAME_LEN {
        out.push(buffer.pop_front().unwrap_or(0));
    }
    out
}

/// Build a packed s16 stereo frame at `rate` from interleaved samples.
fn build_frame(interleaved: &[i16], rate: u32, pts: i64) -> Audio {
    let mut frame = Audio::new(OUT_FMT, FRAME_SAMPLES, ChannelLayout::STEREO);
    frame.set_rate(rate);
    frame.set_pts(Some(pts));
    let data = frame.data_mut(0);
    for (i, &sample) in interleaved.iter().enumerate() {
        let bytes = sample.to_ne_bytes();
        data[i * 2] = bytes[0];
        data[i * 2 + 1] = bytes[1];
    }
    frame
}

#[cfg(test)]
#[path = "audio_mixer_test.rs"]
mod audio_mixer_test;
