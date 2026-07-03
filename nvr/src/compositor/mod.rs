//! Multi-view compositor programs. Each composites several sources into ONE
//! stream (mosaic / video wall / picture-in-picture, via `nvr-compositor`) and
//! publishes it to ZLM; managed through the API.

pub mod api;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use tokio::sync::RwLock;

use nvr_compositor::{Compositor, CompositorConfig, Layout, Region, Source, SourceFeed};

/// ZLM RTMP publish endpoint (see `zlm::server`: `rtmp_server_start(8555)`).
const ZLM_RTMP: &str = "rtmp://127.0.0.1:8555";

#[derive(Clone)]
pub struct SourceInfo {
    pub id: String,
    pub url: String,
}

pub struct CompositorEntry {
    pub id: String,
    pub sources: Vec<SourceInfo>,
    pub layout: Layout,
    pub publish_url: String,
    pub fps: u32,
    compositor: Compositor,
    // Kept alive (decoding) for the lifetime of the entry.
    _sources: Vec<Source>,
}

impl CompositorEntry {
    /// Current active source id per region, by region order.
    pub fn active(&self) -> Vec<String> {
        self.compositor.active()
    }

    /// Switch region `index` to `source_id`; false if out of range or unknown.
    pub fn switch(&self, index: usize, source_id: &str) -> bool {
        self.compositor.switch(index, source_id)
    }
}

pub struct CreateParams {
    pub id: String,
    pub sources: Vec<SourceInfo>,
    pub width: u32,
    pub height: u32,
    /// Explicit regions; empty means auto grid over the sources.
    pub regions: Vec<Region>,
    pub fps: u32,
    pub bitrate: Option<u64>,
    pub publish_url: Option<String>,
}

static COMPOSITORS: LazyLock<RwLock<HashMap<String, Arc<CompositorEntry>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Create and start a compositor, publishing it to ZLM.
pub async fn create(params: CreateParams) -> Result<Arc<CompositorEntry>> {
    let id = params.id.trim().to_string();
    if id.is_empty() {
        anyhow::bail!("compositor id is required");
    }
    if params.sources.is_empty() {
        anyhow::bail!("need at least one source");
    }
    if COMPOSITORS.read().await.contains_key(&id) {
        anyhow::bail!("compositor {id} already exists");
    }

    // Build the layout (explicit regions or auto grid) and validate that every
    // region references a declared source — before starting anything.
    let known: HashSet<&str> = params.sources.iter().map(|s| s.id.as_str()).collect();
    let layout = if params.regions.is_empty() {
        let ids: Vec<String> = params.sources.iter().map(|s| s.id.clone()).collect();
        Layout::grid(params.width, params.height, &ids)
    } else {
        Layout::new(params.width, params.height, params.regions)
    };
    for region in &layout.regions {
        if !known.contains(region.source_id.as_str()) {
            anyhow::bail!("region references unknown source '{}'", region.source_id);
        }
    }

    // Start every source hot.
    let mut started = Vec::with_capacity(params.sources.len());
    for s in &params.sources {
        started.push(Source::start(&s.id, &s.url).await?);
    }
    let template = started[0].video_stream.clone();
    let feeds: Vec<SourceFeed> = started
        .iter()
        .map(|s| SourceFeed {
            id: s.id.clone(),
            latest: s.latest.clone(),
        })
        .collect();

    let publish_url = params
        .publish_url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| format!("{ZLM_RTMP}/live/{id}"));
    let cfg = CompositorConfig {
        publish_url: publish_url.clone(),
        format: "flv".to_string(),
        fps: params.fps,
        bitrate: params.bitrate,
    };
    let compositor = Compositor::start(cfg, layout.clone(), feeds, template);

    let entry = Arc::new(CompositorEntry {
        id: id.clone(),
        sources: params.sources,
        layout,
        publish_url,
        fps: params.fps,
        compositor,
        _sources: started,
    });
    COMPOSITORS.write().await.insert(id, entry.clone());
    Ok(entry)
}

pub async fn list() -> Vec<Arc<CompositorEntry>> {
    COMPOSITORS.read().await.values().cloned().collect()
}

/// Remove and stop a compositor. Cancelling stops the compositing loop (which
/// flushes and stops publishing); dropping the entry stops its sources. Returns
/// false if not found.
pub async fn remove(id: &str) -> bool {
    match COMPOSITORS.write().await.remove(id) {
        Some(entry) => {
            entry.compositor.stop();
            true
        }
        None => false,
    }
}

/// Switch a region of a running compositor to another source in its pool.
pub async fn switch(id: &str, region: usize, source_id: &str) -> Result<()> {
    let entry = COMPOSITORS
        .read()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("compositor {id} not found"))?;
    if !entry.switch(region, source_id) {
        anyhow::bail!(
            "switch rejected: region {region} / source '{source_id}' out of range or not in pool"
        );
    }
    Ok(())
}
