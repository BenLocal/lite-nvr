//! Offline comparison: run one image through every model in a manifest and
//! print a table (box count, mean confidence, inference time).
//!
//!   cargo run -p nvr-detect --example detect-compare -- \
//!     --image path/to.jpg --models third_party/detect-models/models.json \
//!     --models-dir third_party/detect-models

use std::path::{Path, PathBuf};

use clap::Parser;
use nvr_detect::{Detector, DetectorConfig, DetectorSet, UslsDetector};

#[derive(Parser)]
struct Args {
    #[arg(long)]
    image: PathBuf,
    #[arg(long)]
    models: PathBuf,
    #[arg(long, default_value = "third_party/detect-models")]
    models_dir: PathBuf,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let args = Args::parse();

    let manifest = std::fs::read_to_string(&args.models)?;
    let cfgs: Vec<DetectorConfig> = serde_json::from_str(&manifest)?;

    let mut detectors: Vec<Box<dyn Detector>> = Vec::new();
    for cfg in &cfgs {
        let path = resolve(&args.models_dir, &cfg.model_file);
        match UslsDetector::new(cfg, &path) {
            Ok(d) => detectors.push(Box::new(d)),
            Err(e) => eprintln!("skip {}: {e:#}", cfg.name),
        }
    }
    let set = DetectorSet::new(detectors);

    let img = image::open(&args.image)?.to_rgb8();
    let (w, h) = img.dimensions();
    let results = set.detect_all(img.as_raw(), w, h);

    println!(
        "{:<16} {:>6} {:>10} {:>10}",
        "model", "boxes", "mean_conf", "infer_ms"
    );
    for r in &results {
        let mean = if r.detections.is_empty() {
            0.0
        } else {
            r.detections.iter().map(|d| d.confidence).sum::<f32>() / r.detections.len() as f32
        };
        match &r.error {
            Some(e) => println!("{:<16} ERROR {e}", r.name),
            None => println!(
                "{:<16} {:>6} {:>10.3} {:>10.1}",
                r.name,
                r.detections.len(),
                mean,
                r.infer_ms
            ),
        }
    }
    Ok(())
}

fn resolve(dir: &Path, file: &str) -> PathBuf {
    let p = Path::new(file);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        dir.join(p)
    }
}
