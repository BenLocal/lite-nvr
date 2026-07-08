//! Multi-view compositor programs. Each composites several sources into ONE
//! stream (mosaic / video wall / picture-in-picture, via `nvr-compositor`) and
//! publishes it to ZLM; managed through the API.

pub mod api;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use nvr_compositor::{Compositor, CompositorConfig, Layout, Region, Source, SourceFeed};

/// KV config key under which the live compositor programs are persisted so they
/// can be restored at startup.
const PERSIST_KEY: &str = "compositor_programs";

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
    /// Retained so the program can be persisted/restored verbatim.
    bitrate: Option<u64>,
    compositor: Compositor,
    /// Cancels the background reconnect tasks for sources that were offline at
    /// create time, when the program is removed.
    reconnect_cancel: CancellationToken,
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

    /// Replace the region layout live (canvas size unchanged).
    pub fn relayout(&self, layout: &Layout) {
        self.compositor.relayout(layout);
    }

    /// Current region geometry `(x, y, w, h)` per region — reflects the live
    /// layout after any [`relayout`](Self::relayout).
    pub fn geoms(&self) -> Vec<(u32, u32, u32, u32)> {
        self.compositor.geoms()
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
        // An empty source id is a deliberately blank (black) region slot.
        let sid = region.source_id.as_str();
        if !sid.is_empty() && !known.contains(sid) {
            anyhow::bail!("region references unknown source '{}'", region.source_id);
        }
    }

    // Start every source hot. A source that can't start yet (e.g. a camera
    // still coming up, or its ZLM stream not republished after a restart) is
    // NOT dropped: it stays in the switchable pool with an empty frame cell and
    // reconnects in the background, so it appears — and becomes switchable — the
    // moment it comes online (its regions stay black until then). At least one
    // source must start: its stream seeds the encoder.
    let reconnect_cancel = CancellationToken::new();
    let mut started = Vec::with_capacity(params.sources.len());
    let mut feeds: Vec<SourceFeed> = Vec::with_capacity(params.sources.len());
    for s in &params.sources {
        match Source::start(&s.id, &s.url).await {
            Ok(src) => {
                feeds.push(SourceFeed {
                    id: src.id.clone(),
                    latest: src.latest.clone(),
                });
                started.push(src);
            }
            Err(e) => {
                log::warn!(
                    "compositor '{id}' source '{}' offline, keeping in pool and reconnecting: {e:#}",
                    s.id
                );
                let latest =
                    nvr_compositor::spawn_reconnecting(&s.id, &s.url, reconnect_cancel.clone());
                feeds.push(SourceFeed {
                    id: s.id.clone(),
                    latest,
                });
            }
        }
    }
    if started.is_empty() {
        reconnect_cancel.cancel();
        anyhow::bail!("no source could be started");
    }
    let template = started[0].video_stream.clone();

    let publish_url = params
        .publish_url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| format!("{ZLM_RTMP}/switcher/{id}"));
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
        bitrate: params.bitrate,
        compositor,
        reconnect_cancel,
        _sources: started,
    });
    COMPOSITORS.write().await.insert(id, entry.clone());
    persist_all().await;
    Ok(entry)
}

pub async fn list() -> Vec<Arc<CompositorEntry>> {
    COMPOSITORS.read().await.values().cloned().collect()
}

/// Stop every running compositor (compositing loop + source decoders +
/// reconnectors) for a clean process shutdown. Does NOT clear the persisted
/// config, so the programs restore on the next start. Call before the process
/// exits so no compositor thread is still writing into ZLM when its C runtime
/// is torn down.
pub async fn shutdown() {
    let entries: Vec<Arc<CompositorEntry>> =
        { COMPOSITORS.write().await.drain().map(|(_, e)| e).collect() };
    for e in &entries {
        e.compositor.stop();
        e.reconnect_cancel.cancel();
    }
    // `entries` drops here → each entry's sources drop → their input/decoder
    // tasks stop (Source::drop).
}

/// Remove and stop a compositor. Cancelling stops the compositing loop (which
/// flushes and stops publishing); dropping the entry stops its sources. Returns
/// false if not found.
pub async fn remove(id: &str) -> bool {
    // Bind the removal so the write guard drops before persist_all() read-locks.
    let removed = COMPOSITORS.write().await.remove(id);
    match removed {
        Some(entry) => {
            entry.compositor.stop();
            entry.reconnect_cancel.cancel();
            persist_all().await;
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
    persist_all().await;
    Ok(())
}

/// Relayout a running compositor's regions live — no stream restart. The canvas
/// size stays as it was at create; only the region rectangles/sources change.
pub async fn relayout(id: &str, regions: Vec<Region>) -> Result<()> {
    if regions.is_empty() {
        anyhow::bail!("layout has no regions");
    }
    let entry = COMPOSITORS
        .read()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("compositor {id} not found"))?;
    // Validate like create: every region references a declared source (an empty
    // source is a deliberately blank/black region).
    let known: HashSet<&str> = entry.sources.iter().map(|s| s.id.as_str()).collect();
    for region in &regions {
        let sid = region.source_id.as_str();
        if !sid.is_empty() && !known.contains(sid) {
            anyhow::bail!("region references unknown source '{}'", region.source_id);
        }
    }
    let layout = Layout::new(entry.layout.width, entry.layout.height, regions);
    entry.relayout(&layout);
    persist_all().await;
    Ok(())
}

// ---- Persistence ------------------------------------------------------------
// Keep the KV store in sync with the live programs so they restore on the next
// startup.

#[derive(Serialize, Deserialize, Clone)]
struct PersistedRegion {
    source: String,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Serialize, Deserialize, Clone)]
struct PersistedSource {
    id: String,
    url: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct PersistedCompositor {
    id: String,
    sources: Vec<PersistedSource>,
    width: u32,
    height: u32,
    fps: u32,
    #[serde(default)]
    bitrate: Option<u64>,
    #[serde(default)]
    publish_url: Option<String>,
    regions: Vec<PersistedRegion>,
}

impl CompositorEntry {
    /// Snapshot the entry's current state (live geometry + active source per
    /// region) into its persistable form.
    fn to_persisted(&self) -> PersistedCompositor {
        let regions = self
            .geoms()
            .into_iter()
            .zip(self.active())
            .map(|((x, y, w, h), source)| PersistedRegion { source, x, y, w, h })
            .collect();
        PersistedCompositor {
            id: self.id.clone(),
            sources: self
                .sources
                .iter()
                .map(|s| PersistedSource {
                    id: s.id.clone(),
                    url: s.url.clone(),
                })
                .collect(),
            width: self.layout.width,
            height: self.layout.height,
            fps: self.fps,
            bitrate: self.bitrate,
            publish_url: Some(self.publish_url.clone()),
            regions,
        }
    }
}

/// Rewrite a pre-rename persisted URL onto the current ZLM app layout so old
/// saved programs restore correctly instead of pulling/publishing dead `live`
/// streams: device source pulls moved from the `live` app to `device` (RTSP,
/// port 8554), and the program publish from `live` to `switcher` (RTMP, port
/// 8555). Port-anchored so only local ZLM URLs are touched — a no-op for URLs
/// already on the new apps or on other apps (e.g. GB28181 `rtp` pulls).
fn migrate_url(url: &str) -> String {
    url.replace(":8554/live/", ":8554/device/")
        .replace(":8555/live/", ":8555/switcher/")
}

impl PersistedCompositor {
    fn into_params(self) -> CreateParams {
        CreateParams {
            id: self.id,
            sources: self
                .sources
                .into_iter()
                .map(|s| SourceInfo { id: s.id, url: migrate_url(&s.url) })
                .collect(),
            width: self.width,
            height: self.height,
            regions: self
                .regions
                .into_iter()
                .map(|r| Region {
                    source_id: r.source,
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                })
                .collect(),
            fps: self.fps,
            bitrate: self.bitrate,
            publish_url: self.publish_url.as_deref().map(migrate_url),
        }
    }
}

/// Serialize every live program to the KV store. Best-effort: a failure is
/// logged but never breaks the live operation that triggered it.
async fn persist_all() {
    let list: Vec<PersistedCompositor> = {
        let guard = COMPOSITORS.read().await;
        guard.values().map(|e| e.to_persisted()).collect()
    };
    if let Err(e) = save_persisted(list).await {
        log::warn!("compositor: failed to persist programs: {e:#}");
    }
}

async fn save_persisted(list: Vec<PersistedCompositor>) -> Result<()> {
    let conn = crate::db::app_db_conn()?;
    nvr_db::config::set_json(PERSIST_KEY, &list, &conn).await?;
    Ok(())
}

async fn load_persisted() -> Result<Vec<PersistedCompositor>> {
    let conn = crate::db::app_db_conn()?;
    Ok(
        nvr_db::config::get_json::<Vec<PersistedCompositor>>(PERSIST_KEY, &conn)
            .await?
            .unwrap_or_default(),
    )
}

/// Restore persisted compositor programs at startup. Call AFTER the device pipes
/// have started so the sources' ZLM streams exist (compositors pull
/// `rtsp://127.0.0.1:8554/device/{id}`). Best-effort, with a short grace period
/// and a few retries since streams may still be coming up.
pub async fn restore_all() {
    let saved = match load_persisted().await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("compositor restore: failed to load saved programs: {e:#}");
            return;
        }
    };
    if saved.is_empty() {
        return;
    }
    log::info!("compositor restore: {} program(s) to restore", saved.len());
    // Give device pipelines a moment to publish their streams to ZLM first.
    tokio::time::sleep(Duration::from_secs(3)).await;

    for pc in saved {
        let id = pc.id.clone();
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match create(pc.clone().into_params()).await {
                Ok(_) => {
                    log::info!("compositor restore: started '{id}'");
                    break;
                }
                Err(e) if attempt < 3 => {
                    log::warn!(
                        "compositor restore: '{id}' attempt {attempt} failed ({e:#}); retrying"
                    );
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(e) => {
                    log::error!(
                        "compositor restore: '{id}' gave up after {attempt} attempts: {e:#}"
                    );
                    break;
                }
            }
        }
    }
}
