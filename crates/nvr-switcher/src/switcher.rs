//! Orchestration: starts every source hot, spawns the program bus, and exposes
//! `switch(id)` — an atomic change of the active source plus a force-IDR flag.
//! The program's encoder/muxer are untouched by a switch, so the player never
//! sees an interruption.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::mpsc;

use crate::program::{ProgramConfig, spawn_program};
use crate::source::{Source, TaggedFrame};

/// The currently selected source id, shared between the control side and the
/// program loop.
pub type Active = Arc<Mutex<String>>;

pub struct Switcher {
    active: Active,
    force_idr: Arc<AtomicBool>,
    ids: Vec<String>,
    // Kept alive for the lifetime of the switcher.
    _sources: Vec<Source>,
    _program: tokio::task::JoinHandle<Result<()>>,
}

impl Switcher {
    /// Start all `sources` (`(id, url)`) hot and begin publishing the program
    /// (initially the first source) via `cfg`.
    pub async fn start(sources: Vec<(String, String)>, cfg: ProgramConfig) -> Result<Self> {
        if sources.is_empty() {
            anyhow::bail!("need at least one source");
        }

        // ~2 seconds of buffering across all sources; sources drop on full.
        let cap = (cfg.fps as usize * 2).max(8);
        let (tx, rx) = mpsc::channel::<TaggedFrame>(cap);

        let mut started = Vec::with_capacity(sources.len());
        let mut ids = Vec::with_capacity(sources.len());
        for (id, url) in &sources {
            if ids.iter().any(|x| x == id) {
                anyhow::bail!("duplicate source id: {id}");
            }
            let source = Source::start(id, url, tx.clone()).await?;
            ids.push(id.clone());
            started.push(source);
        }
        drop(tx); // each source holds its own sender clone

        let template = started[0].video_stream.clone();
        let active: Active = Arc::new(Mutex::new(ids[0].clone()));
        let force_idr = Arc::new(AtomicBool::new(true)); // first frame is an IDR
        let program = spawn_program(cfg, template, active.clone(), force_idr.clone(), rx);

        Ok(Self {
            active,
            force_idr,
            ids,
            _sources: started,
            _program: program,
        })
    }

    /// Switch the program to source `id`. Returns false if `id` is unknown.
    /// The output stream is unaffected — playback continues without a break.
    pub fn switch(&self, id: &str) -> bool {
        if !self.ids.iter().any(|x| x == id) {
            return false;
        }
        *self.active.lock().unwrap() = id.to_string();
        self.force_idr.store(true, Ordering::Release);
        true
    }

    pub fn ids(&self) -> &[String] {
        &self.ids
    }

    /// The currently active (program) source id.
    pub fn active(&self) -> String {
        self.active.lock().unwrap().clone()
    }
}
