//! usls (ONNX Runtime) YOLO backend. Wraps `usls::models::YOLO` behind a
//! `Mutex` so `detect(&self)` satisfies the `Detector` trait even though usls
//! inference needs `&mut self`; contention is nil because each detector runs at
//! most once per sampled frame.

use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use usls::models::YOLO;
use usls::{Config, Device, Image, Scale, Version};

use crate::coco;
use crate::config::DetectorConfig;
use crate::detector::Detector;
use crate::types::{BBox, Detection};

pub struct UslsDetector {
    name: String,
    model: Mutex<YOLO>,
}

impl UslsDetector {
    /// Build a detector from config. `model_path` is the resolved absolute path
    /// to the `.onnx` weights.
    pub fn new(cfg: &DetectorConfig, model_path: &Path) -> anyhow::Result<Self> {
        let path = model_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-utf8 model path"))?;

        let names = if cfg.class_names.is_empty() {
            coco::default_names()
        } else {
            cfg.class_names.clone()
        };
        let n = names.len();
        let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();

        let device = Device::from_str(&cfg.device)
            .map_err(|e| anyhow::anyhow!("invalid device {:?}: {e}", cfg.device))?;

        let mut config = Config::yolo_detect()
            .with_model_file(path)
            .with_class_confs(&vec![cfg.conf; n])
            .with_class_names(&name_refs)
            .with_model_device(device)
            .with_iou(cfg.iou);
        // `version`/`scale` are optional usls hints. `input_size` is a manifest
        // hint: usls reads the real input dims from the ONNX model, so it is
        // advisory and not applied here. Note: usls requires `version` to be
        // set for object detection (it cannot infer the output layout from the
        // ONNX file alone), so a manifest entry with no `version` will fail at
        // `commit()`/`YOLO::new()` below with a clear usls error.
        log::debug!(
            "detector {}: conf={} iou={} input_size={} device={}",
            cfg.name,
            cfg.conf,
            cfg.iou,
            cfg.input_size,
            cfg.device
        );
        if let Some(v) = cfg.version {
            let version =
                Version::try_from(v).map_err(|e| anyhow::anyhow!("invalid version {v}: {e}"))?;
            config = config.with_version(version);
        }
        if let Some(scale) = &cfg.scale {
            let scale = Scale::from_str(scale)
                .map_err(|e| anyhow::anyhow!("invalid scale {scale:?}: {e}"))?;
            config = config.with_scale(scale);
        }
        let config = config.commit()?;

        let model = YOLO::new(config)?;
        Ok(Self {
            name: cfg.name.clone(),
            model: Mutex::new(model),
        })
    }
}

impl Detector for UslsDetector {
    fn name(&self) -> &str {
        &self.name
    }

    fn detect(&self, rgb: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<Detection>> {
        let image = Image::from_u8s(rgb, width, height)?;
        let mut model = self
            .model
            .lock()
            .map_err(|_| anyhow::anyhow!("detector mutex poisoned"))?;
        let ys = model.forward(&[image])?;
        let y = ys
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("usls returned no output"))?;

        let mut out = Vec::new();
        for hbb in y.hbbs() {
            out.push(Detection {
                class_id: hbb.id().unwrap_or(0),
                label: hbb.name().unwrap_or("").to_string(),
                bbox: BBox {
                    x1: hbb.xmin(),
                    y1: hbb.ymin(),
                    x2: hbb.xmax(),
                    y2: hbb.ymax(),
                },
                confidence: hbb.confidence().unwrap_or(0.0),
            });
        }
        Ok(out)
    }
}
