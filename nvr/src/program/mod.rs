//! Director/switcher programs. Each program decodes several sources hot and
//! publishes ONE seamless stream to ZLM (via `nvr-switcher`); the active source
//! can be switched at runtime through the API without interrupting the player.

pub mod api;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use tokio::sync::RwLock;

use nvr_switcher::{ProgramConfig, Switcher};

/// ZLM RTMP publish endpoint (see `zlm::server`: `rtmp_server_start(8555)`).
const ZLM_RTMP: &str = "rtmp://127.0.0.1:8555";

#[derive(Clone)]
pub struct SourceInfo {
    pub id: String,
    pub url: String,
}

pub struct ProgramEntry {
    pub id: String,
    pub sources: Vec<SourceInfo>,
    pub publish_url: String,
    pub fps: u32,
    switcher: Switcher,
}

impl ProgramEntry {
    /// The currently active source id.
    pub fn active(&self) -> String {
        self.switcher.active()
    }
}

pub struct CreateParams {
    pub id: String,
    pub sources: Vec<SourceInfo>,
    pub fps: u32,
    pub bitrate: Option<u64>,
    /// Publish URL override; default `rtmp://127.0.0.1:8555/switcher/{id}`.
    pub publish_url: Option<String>,
}

static PROGRAMS: LazyLock<RwLock<HashMap<String, Arc<ProgramEntry>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Create and start a program, publishing it to ZLM.
pub async fn create(params: CreateParams) -> Result<Arc<ProgramEntry>> {
    let id = params.id.trim().to_string();
    if id.is_empty() {
        anyhow::bail!("program id is required");
    }
    if params.sources.is_empty() {
        anyhow::bail!("need at least one source");
    }
    if PROGRAMS.read().await.contains_key(&id) {
        anyhow::bail!("program {id} already exists");
    }

    let publish_url = params
        .publish_url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| format!("{ZLM_RTMP}/switcher/{id}"));
    let cfg = ProgramConfig {
        publish_url: publish_url.clone(),
        format: "flv".to_string(),
        fps: params.fps,
        bitrate: params.bitrate,
    };
    let source_pairs: Vec<(String, String)> = params
        .sources
        .iter()
        .map(|s| (s.id.clone(), s.url.clone()))
        .collect();

    let switcher = Switcher::start(source_pairs, cfg).await?;
    let entry = Arc::new(ProgramEntry {
        id: id.clone(),
        sources: params.sources,
        publish_url,
        fps: params.fps,
        switcher,
    });
    PROGRAMS.write().await.insert(id, entry.clone());
    Ok(entry)
}

/// Switch a program's active source. Errors if the program or source is unknown.
pub async fn switch(program_id: &str, source_id: &str) -> Result<()> {
    let entry = PROGRAMS
        .read()
        .await
        .get(program_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("program {program_id} not found"))?;
    if !entry.switcher.switch(source_id) {
        anyhow::bail!("program {program_id} has no source '{source_id}'");
    }
    Ok(())
}

pub async fn list() -> Vec<Arc<ProgramEntry>> {
    PROGRAMS.read().await.values().cloned().collect()
}

/// Stop every running program (each program's `Switcher`: its hot sources plus
/// the program loop that publishes to ZLM) for a clean process shutdown.
/// Programs are not persisted, so there is nothing to preserve. Call before the
/// process exits so no program thread is still writing into ZLM when its C
/// runtime is torn down.
pub async fn shutdown() {
    // Draining drops every entry — the same teardown `remove` relies on, done
    // for all programs at once. Each dropped `ProgramEntry` drops its `Switcher`,
    // which drops its sources (`Source::drop` stops the input/decoder tasks); the
    // program loop's input channel then closes, ending the loop and stopping the
    // publish to ZLM. Collect out of the lock so the drops run after the guard.
    let entries: Vec<Arc<ProgramEntry>> =
        { PROGRAMS.write().await.drain().map(|(_, e)| e).collect() };
    drop(entries);
}

/// Remove and stop a program. Dropping the entry stops its sources, which ends
/// the program task and stops publishing to ZLM. Returns false if not found.
pub async fn remove(program_id: &str) -> bool {
    PROGRAMS.write().await.remove(program_id).is_some()
}
