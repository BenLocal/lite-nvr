//! Offline text-to-speech via [sherpa-onnx].
//!
//! [`Tts`] loads a model once and synthesizes text to mono `f32` PCM. For
//! streamed text (an LLM reply), feed it through a [`SentenceSplitter`] and
//! synthesize each sentence as it completes, so speech starts before the
//! whole reply exists. Text is cleaned of Markdown, links and emoji first.
//!
//! Backends, all producing the same kind of [`Audio`]:
//! - sherpa-onnx on the CPU: `vits-melo-tts-zh_en` (Chinese with English
//!   words) or `matcha-icefall-zh-baker` (smaller, Chinese only);
//! - [`ModelKind::MeloRknn`] on Rockchip: the same MeloTTS voice with its
//!   decoder on the NPU, about 5x faster than real time where the CPU
//!   backend barely keeps up.
//!
//! [`Tts::load_first`] picks the first backend that loads, so a Rockchip
//! build can prefer the NPU and fall back to the CPU.
//!
//! [sherpa-onnx]: https://github.com/k2-fsa/sherpa-onnx

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsMatchaModelConfig,
    OfflineTtsModelConfig, OfflineTtsVitsModelConfig,
};

mod melo;
mod melo_rknn;
mod numbers;
mod rknn;
mod text;

// Links the libstdc++ shim for older cross toolchains.
use nvr_sherpa_compat as _;

pub use text::{SentenceSplitter, clean};

/// Model family; decides which files are read from the model directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelKind {
    /// `model.onnx`, `lexicon.txt`, `tokens.txt`, optional `dict/`.
    Vits,
    /// `model-steps-3.onnx` with a separate vocoder `vocos-22khz-univ.onnx`.
    Matcha,
    /// MeloTTS for Rockchip NPUs: `encoder.onnx`, `decoder.rknn`, `g.bin`,
    /// `lexicon.txt`, `tokens.txt`. Needs `librknnrt.so` and onnxruntime
    /// (`ORT_DYLIB_PATH`).
    MeloRknn,
}

impl std::str::FromStr for ModelKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.trim() {
            "vits" => Ok(Self::Vits),
            "matcha" => Ok(Self::Matcha),
            "melo-rknn" => Ok(Self::MeloRknn),
            other => anyhow::bail!(
                "unknown TTS model kind {other:?} (expected vits, matcha or melo-rknn)"
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TtsConfig {
    pub model_dir: PathBuf,
    pub kind: ModelKind,
    /// VITS weights file in `model_dir`, default `model.onnx`. The int8
    /// export is smaller but measured 3-4x slower on both x86 and RK3588.
    pub model_file: Option<String>,
    /// Voice for multi-speaker models.
    pub speaker_id: i32,
    /// 1.0 is normal speed; larger is faster.
    pub speed: f32,
    pub num_threads: i32,
    /// `librknnrt.so` path for [`ModelKind::MeloRknn`]; a bare name is
    /// searched on `LD_LIBRARY_PATH`.
    pub rknn_lib: String,
}

impl TtsConfig {
    pub fn new(model_dir: impl Into<PathBuf>, kind: ModelKind) -> Self {
        Self {
            model_dir: model_dir.into(),
            kind,
            model_file: None,
            speaker_id: 0,
            speed: 1.0,
            num_threads: 4,
            rknn_lib: "librknnrt.so".into(),
        }
    }
}

/// Synthesized mono audio.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Audio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl Audio {
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / self.sample_rate as f32
    }

    /// 16-bit PCM WAV.
    pub fn write_wav(&self, path: &Path) -> Result<()> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec)?;
        for &s in &self.samples {
            let v = (s.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round();
            // In range after the clamp.
            writer.write_sample(v as i16)?;
        }
        writer.finalize()?;
        Ok(())
    }
}

enum Engine {
    Sherpa {
        tts: OfflineTts,
        generation: GenerationConfig,
    },
    // The encoder session and NPU context run one request at a time.
    MeloRknn(Mutex<melo_rknn::MeloRknn>),
}

/// A loaded TTS model. `synthesize` is blocking and CPU-heavy: call it from a
/// blocking thread.
pub struct Tts {
    engine: Engine,
}

fn required(dir: &Path, name: &str) -> Result<String> {
    let path = dir.join(name);
    if !path.is_file() {
        anyhow::bail!("TTS model file missing: {}", path.display());
    }
    Ok(path.to_string_lossy().into_owned())
}

fn optional(dir: &Path, name: &str) -> Option<String> {
    let path = dir.join(name);
    path.exists().then(|| path.to_string_lossy().into_owned())
}

/// Text-normalization FSTs shipped with the Chinese models (dates, numbers,
/// phone numbers, polyphones), in the order sherpa-onnx applies them.
fn rule_fsts(dir: &Path) -> Option<String> {
    let found: Vec<String> = ["date.fst", "phone.fst", "number.fst", "new_heteronym.fst"]
        .iter()
        .filter_map(|name| optional(dir, name))
        .collect();
    (!found.is_empty()).then(|| found.join(","))
}

impl Tts {
    /// Load the first config that works, logging why earlier ones did not.
    pub fn load_first(configs: &[TtsConfig]) -> Result<Self> {
        let mut failures = Vec::new();
        for cfg in configs {
            match Self::load(cfg) {
                Ok(tts) => {
                    log::info!("tts: using {:?} from {}", cfg.kind, cfg.model_dir.display());
                    return Ok(tts);
                }
                Err(err) => {
                    log::warn!(
                        "tts: {:?} from {} unavailable: {err:#}",
                        cfg.kind,
                        cfg.model_dir.display()
                    );
                    failures.push(format!("{:?}: {err:#}", cfg.kind));
                }
            }
        }
        anyhow::bail!("no TTS backend could be loaded ({})", failures.join("; "))
    }

    pub fn load(cfg: &TtsConfig) -> Result<Self> {
        let dir = &cfg.model_dir;
        if cfg.kind == ModelKind::MeloRknn {
            let melo = melo_rknn::MeloRknn::load(dir, &cfg.rknn_lib, cfg.speed)?;
            return Ok(Self {
                engine: Engine::MeloRknn(Mutex::new(melo)),
            });
        }
        let mut model = OfflineTtsModelConfig {
            num_threads: cfg.num_threads.max(1),
            ..Default::default()
        };
        match cfg.kind {
            ModelKind::Vits => {
                model.vits = OfflineTtsVitsModelConfig {
                    model: Some(required(
                        dir,
                        cfg.model_file.as_deref().unwrap_or("model.onnx"),
                    )?),
                    lexicon: Some(required(dir, "lexicon.txt")?),
                    tokens: Some(required(dir, "tokens.txt")?),
                    dict_dir: optional(dir, "dict"),
                    ..Default::default()
                };
            }
            ModelKind::Matcha => {
                model.matcha = OfflineTtsMatchaModelConfig {
                    acoustic_model: Some(required(dir, "model-steps-3.onnx")?),
                    vocoder: Some(required(dir, "vocos-22khz-univ.onnx")?),
                    lexicon: Some(required(dir, "lexicon.txt")?),
                    tokens: Some(required(dir, "tokens.txt")?),
                    dict_dir: optional(dir, "dict"),
                    ..Default::default()
                };
            }
            ModelKind::MeloRknn => unreachable!("handled above"),
        }
        let config = OfflineTtsConfig {
            model,
            rule_fsts: rule_fsts(dir),
            max_num_sentences: 1,
            ..Default::default()
        };
        let tts = OfflineTts::create(&config)
            .with_context(|| format!("sherpa-onnx rejected the TTS model in {}", dir.display()))?;
        Ok(Self {
            engine: Engine::Sherpa {
                tts,
                generation: GenerationConfig {
                    sid: cfg.speaker_id,
                    speed: cfg.speed,
                    ..Default::default()
                },
            },
        })
    }

    pub fn sample_rate(&self) -> u32 {
        match &self.engine {
            Engine::Sherpa { tts, .. } => u32::try_from(tts.sample_rate()).unwrap_or(0),
            Engine::MeloRknn(_) => melo_rknn::SAMPLE_RATE,
        }
    }

    /// Clean and synthesize `text`; empty audio when nothing is speakable.
    pub fn synthesize(&self, text: &str) -> Result<Audio> {
        let text = clean(text);
        if !text.chars().any(char::is_alphanumeric) {
            return Ok(Audio {
                samples: Vec::new(),
                sample_rate: self.sample_rate(),
            });
        }
        match &self.engine {
            Engine::Sherpa { tts, generation } => {
                let audio = tts
                    .generate_with_config(&text, generation, None::<fn(&[f32], f32) -> bool>)
                    .context("TTS generation failed")?;
                Ok(Audio {
                    samples: audio.samples().to_vec(),
                    sample_rate: u32::try_from(audio.sample_rate()).unwrap_or(0),
                })
            }
            Engine::MeloRknn(melo) => {
                let samples = melo
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .synthesize(&text)?;
                Ok(Audio {
                    samples,
                    sample_rate: melo_rknn::SAMPLE_RATE,
                })
            }
        }
    }
}

#[cfg(test)]
#[path = "lib_test.rs"]
mod lib_test;
