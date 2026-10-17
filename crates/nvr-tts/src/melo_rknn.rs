//! MeloTTS split for Rockchip: `encoder.onnx` (text to latent frames) on the
//! CPU through onnxruntime, `decoder.rknn` (latent frames to 44.1 kHz audio)
//! on the NPU. Model files: happyme531/MeloTTS-RKNN2 (AGPL-3.0, fetched by
//! `scripts/download_tts_models.sh --melo-rknn`, not vendored).
//!
//! The NPU decoder has a fixed 128-frame input, so the latent sequence is
//! decoded in windows that overlap by 32 frames and are cross-faded.

use std::path::Path;

use anyhow::{Context, Result};
use ort::session::Session;
use ort::value::Tensor;

use crate::melo::Lexicon;
use crate::rknn;

pub(crate) const SAMPLE_RATE: u32 = 44_100;
/// Latent channels per frame.
const CHANNELS: usize = 192;
/// Audio samples per latent frame.
const SAMPLES_PER_FRAME: usize = 512;
/// Decoder input length in frames, fixed when the model was compiled.
const WINDOW: usize = 128;
const OVERLAP: usize = 32;
/// Speaker embedding length (`g.bin`).
const SPEAKER_DIM: usize = 256;
/// Language id the model uses for Chinese (English words included).
const LANGUAGE_ZH: i32 = 3;
/// Standard VITS sampling noise: natural prosody without the buzz that
/// zero noise causes on voiced sounds.
const NOISE_SCALE: f32 = 0.667;
const NOISE_SCALE_W: f32 = 0.8;
const SDP_RATIO: f32 = 0.2;
/// The encoder is light (a sentence takes tens of ms); more threads only
/// compete with the rest of nvr.
const ENCODER_THREADS: usize = 2;

pub(crate) struct MeloRknn {
    lexicon: Lexicon,
    encoder: Session,
    decoder: rknn::Model,
    speaker: Vec<f32>,
    length_scale: f32,
}

impl MeloRknn {
    pub(crate) fn load(dir: &Path, rknn_lib: &str, speed: f32) -> Result<Self> {
        let lexicon = Lexicon::load(dir)?;
        let speaker = std::fs::read(dir.join("g.bin"))
            .with_context(|| format!("read {}", dir.join("g.bin").display()))?;
        if speaker.len() != SPEAKER_DIM * 4 {
            anyhow::bail!(
                "g.bin is {} bytes, expected {}",
                speaker.len(),
                SPEAKER_DIM * 4
            );
        }
        let speaker = speaker
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        // Core 0 alone decodes about 6x faster than real time and leaves the
        // other cores to the LLM.
        let decoder = rknn::Model::load(rknn_lib, &dir.join("decoder.rknn"), rknn::CORE_0)?;
        let encoder = Session::builder()
            .and_then(|b| b.with_intra_threads(ENCODER_THREADS))
            .and_then(|b| b.commit_from_file(dir.join("encoder.onnx")))
            .map_err(|e| anyhow::anyhow!("load {}: {e}", dir.join("encoder.onnx").display()))?;
        Ok(Self {
            lexicon,
            encoder,
            decoder,
            speaker,
            length_scale: 1.0 / speed.max(0.1),
        })
    }

    pub(crate) fn synthesize(&mut self, text: &str) -> Result<Vec<f32>> {
        let ids = self.lexicon.phones(text);
        let n = ids.phones.len();
        let latent = self.encode(ids.phones, ids.tones, n)?;
        let frames = latent.len() / CHANNELS;
        let mut speaker = self.speaker.clone();
        let decoder = &mut self.decoder;
        overlap_add(&latent, frames, |window| {
            decoder.run(&mut [window, &mut speaker])
        })
    }

    /// Latent frames `[CHANNELS, frames]`, channel-major.
    fn encode(&mut self, phones: Vec<i32>, tones: Vec<i32>, n: usize) -> Result<Vec<f32>> {
        let err = |e: ort::Error| anyhow::anyhow!("encoder: {e}");
        let scalar = |v: f32| Tensor::from_array(([1usize], vec![v])).map_err(err);
        let inputs = ort::inputs! {
            "phone" => Tensor::from_array(([n], phones)).map_err(err)?,
            "tone" => Tensor::from_array(([n], tones)).map_err(err)?,
            "language" => Tensor::from_array(([n], vec![LANGUAGE_ZH; n])).map_err(err)?,
            "g" => Tensor::from_array(([1usize, SPEAKER_DIM, 1], self.speaker.clone())).map_err(err)?,
            "noise_scale" => scalar(NOISE_SCALE)?,
            "noise_scale_w" => scalar(NOISE_SCALE_W)?,
            "length_scale" => scalar(self.length_scale)?,
            "sdp_ratio" => scalar(SDP_RATIO)?,
        };
        let outputs = self.encoder.run(inputs).map_err(err)?;
        let (shape, data) = outputs["z_p"].try_extract_tensor::<f32>().map_err(err)?;
        if shape.len() != 3 || shape[1] != CHANNELS as i64 {
            anyhow::bail!("encoder z_p has shape {shape:?}");
        }
        Ok(data.to_vec())
    }
}

/// Decode `frames` latent frames (`[CHANNELS, frames]`) through a fixed
/// `WINDOW`-frame decoder: windows advance by `WINDOW - OVERLAP` frames, the
/// tail is zero-padded, and each overlap is linearly cross-faded.
pub(crate) fn overlap_add(
    latent: &[f32],
    frames: usize,
    mut decode: impl FnMut(&mut [f32]) -> Result<Vec<f32>>,
) -> Result<Vec<f32>> {
    if latent.len() < CHANNELS * frames {
        anyhow::bail!(
            "latent has {} values, expected {}",
            latent.len(),
            CHANNELS * frames
        );
    }
    let hop = WINDOW - OVERLAP;
    let fade = OVERLAP * SAMPLES_PER_FRAME;
    let mut audio = vec![0.0f32; frames * SAMPLES_PER_FRAME];
    let mut window = vec![0.0f32; CHANNELS * WINDOW];
    let mut start = 0;
    while start < frames {
        for c in 0..CHANNELS {
            for k in 0..WINDOW {
                let f = start + k;
                window[c * WINDOW + k] = if f < frames {
                    latent[c * frames + f]
                } else {
                    0.0
                };
            }
        }
        let out = decode(&mut window)?;
        if out.len() != WINDOW * SAMPLES_PER_FRAME {
            anyhow::bail!(
                "decoder returned {} samples, expected {}",
                out.len(),
                WINDOW * SAMPLES_PER_FRAME
            );
        }
        let base = start * SAMPLES_PER_FRAME;
        for (i, &y) in out.iter().enumerate() {
            let Some(slot) = audio.get_mut(base + i) else {
                break;
            };
            *slot = if start > 0 && i < fade {
                let a = i as f32 / fade as f32;
                *slot * (1.0 - a) + y * a
            } else {
                y
            };
        }
        if start + WINDOW >= frames {
            break;
        }
        start += hop;
    }
    Ok(audio)
}

#[cfg(test)]
#[path = "melo_rknn_test.rs"]
mod melo_rknn_test;
