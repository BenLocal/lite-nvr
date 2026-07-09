//! Multi-view compositor programs. Each composites several sources into ONE
//! stream (mosaic / video wall / picture-in-picture, via `nvr-compositor`) and
//! publishes it to ZLM; managed through the API.

pub mod api;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
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

/// Grace period before the first restore attempt, letting device pipelines
/// publish their ZLM streams.
const RESTORE_GRACE: Duration = Duration::from_secs(3);
/// Restore retry backoff bounds and total budget: retry `create()` with
/// exponential backoff from MIN to MAX until BUDGET has elapsed. With sources
/// now self-healing, `create()` only hard-fails while ALL of a program's
/// sources are still offline, so a slow boot just needs a wider retry window —
/// its remaining offline sources then reconnect on their own.
const RESTORE_RETRY_MIN: Duration = Duration::from_secs(2);
const RESTORE_RETRY_MAX: Duration = Duration::from_secs(8);
const RESTORE_RETRY_BUDGET: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct SourceInfo {
    pub id: String,
    pub url: String,
}

/// One source of a running program plus the handle that keeps it alive and lets
/// it be removed on its own, without disturbing the other sources.
struct SourceHandle {
    info: SourceInfo,
    /// Cancels THIS source's reconnect loop. For an offline source it drives the
    /// [`nvr_compositor::spawn_reconnecting`] loop; for an online source the loop
    /// is owned by the `Source` (stopped when `_src` drops), so firing this token
    /// is a harmless no-op there.
    cancel: CancellationToken,
    /// `Some` for a source that started online — kept decoding, and stops its own
    /// reconnect loop on drop. `None` for one that was offline at add time (driven
    /// purely by `cancel` + `spawn_reconnecting`).
    _src: Option<Source>,
}

pub struct CompositorEntry {
    pub id: String,
    /// The program's sources, mutable behind a lock so they can be added/removed
    /// live (the entry itself is shared as an `Arc`).
    sources: Mutex<Vec<SourceHandle>>,
    pub layout: Layout,
    pub publish_url: String,
    pub fps: u32,
    /// Retained so the program can be persisted/restored verbatim.
    bitrate: Option<u64>,
    compositor: Compositor,
}

impl CompositorEntry {
    /// The program's sources (id + url), by add order. Locks the source list.
    pub fn source_infos(&self) -> Vec<SourceInfo> {
        self.sources
            .lock()
            .unwrap()
            .iter()
            .map(|h| h.info.clone())
            .collect()
    }

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
    // Each source gets its OWN cancel token, so one source can later be removed
    // without disturbing the others. Online → keep the `Source` alive (its drop
    // stops its loop); offline → drive its `spawn_reconnecting` loop by its token
    // and leave it in the switchable pool (black tile) until it comes up.
    let mut handles: Vec<SourceHandle> = Vec::with_capacity(params.sources.len());
    let mut feeds: Vec<SourceFeed> = Vec::with_capacity(params.sources.len());
    let mut template = None;
    for s in &params.sources {
        match Source::start(&s.id, &s.url).await {
            Ok(src) => {
                template.get_or_insert_with(|| src.video_stream.clone());
                feeds.push(SourceFeed {
                    id: src.id.clone(),
                    latest: src.latest.clone(),
                });
                handles.push(SourceHandle {
                    info: s.clone(),
                    cancel: CancellationToken::new(),
                    _src: Some(src),
                });
            }
            Err(e) => {
                log::warn!(
                    "compositor '{id}' source '{}' offline, keeping in pool and reconnecting: {e:#}",
                    s.id
                );
                let cancel = CancellationToken::new();
                let latest = nvr_compositor::spawn_reconnecting(&s.id, &s.url, cancel.clone());
                feeds.push(SourceFeed {
                    id: s.id.clone(),
                    latest,
                });
                handles.push(SourceHandle {
                    info: s.clone(),
                    cancel,
                    _src: None,
                });
            }
        }
    }
    // At least one source must start: its stream seeds the encoder template.
    let template = match template {
        Some(t) => t,
        None => {
            for h in &handles {
                h.cancel.cancel();
            }
            anyhow::bail!("no source could be started");
        }
    };

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
        sources: Mutex::new(handles),
        layout,
        publish_url,
        fps: params.fps,
        bitrate: params.bitrate,
        compositor,
    });
    COMPOSITORS.write().await.insert(id, entry.clone());
    persist_all().await;
    Ok(entry)
}

/// Fetch a running program by id (e.g. to build a response after a mutation).
pub async fn get(id: &str) -> Option<Arc<CompositorEntry>> {
    COMPOSITORS.read().await.get(id).cloned()
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
        // Cancel every source's reconnect loop (offline handles); online sources
        // also stop when the entry (and thus its `Source`s) drop below.
        for h in e.sources.lock().unwrap().iter() {
            h.cancel.cancel();
        }
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
            for h in entry.sources.lock().unwrap().iter() {
                h.cancel.cancel();
            }
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
    let infos = entry.source_infos();
    let known: HashSet<&str> = infos.iter().map(|s| s.id.as_str()).collect();
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

/// Add a source to a running program's pool, live — the published stream keeps
/// flowing; the source becomes switchable at once (its picture appears once it
/// decodes a frame). Errors if the program is missing or `src.id` already
/// exists in the program.
pub async fn add_source(program_id: &str, src: SourceInfo) -> Result<()> {
    let entry = COMPOSITORS
        .read()
        .await
        .get(program_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("compositor {program_id} not found"))?;
    if entry
        .sources
        .lock()
        .unwrap()
        .iter()
        .any(|h| h.info.id == src.id)
    {
        anyhow::bail!("source '{}' already in compositor '{program_id}'", src.id);
    }

    // Start it hot; if it can't come up yet, keep it in the pool and reconnect in
    // the background (a black tile until it appears) — same policy as `create`.
    let handle = match Source::start(&src.id, &src.url).await {
        Ok(source) => {
            entry
                .compositor
                .add_source(src.id.clone(), source.latest.clone());
            SourceHandle {
                info: src.clone(),
                cancel: CancellationToken::new(),
                _src: Some(source),
            }
        }
        Err(e) => {
            log::warn!(
                "compositor '{program_id}' source '{}' offline, keeping in pool and reconnecting: {e:#}",
                src.id
            );
            let cancel = CancellationToken::new();
            let latest = nvr_compositor::spawn_reconnecting(&src.id, &src.url, cancel.clone());
            entry.compositor.add_source(src.id.clone(), latest);
            SourceHandle {
                info: src.clone(),
                cancel,
                _src: None,
            }
        }
    };
    entry.sources.lock().unwrap().push(handle);
    persist_all().await;
    Ok(())
}

/// Remove a source from a running program's pool, live — the published stream
/// keeps flowing and the source is cleared out of any region it occupies (to
/// black). Errors if the program/source is missing, or if it is the program's
/// last source (remove the program instead).
pub async fn remove_source(program_id: &str, source_id: &str) -> Result<()> {
    let entry = COMPOSITORS
        .read()
        .await
        .get(program_id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("compositor {program_id} not found"))?;
    {
        let mut guard = entry.sources.lock().unwrap();
        let Some(pos) = guard.iter().position(|h| h.info.id == source_id) else {
            anyhow::bail!("source '{source_id}' not found in compositor '{program_id}'");
        };
        if guard.len() == 1 {
            anyhow::bail!("cannot remove the last source; remove the program instead");
        }
        // Drop it from the live pool + any region slots it holds BEFORE dropping
        // its feed, so it can't linger on screen.
        entry.compositor.remove_source(source_id);
        let handle = guard.remove(pos);
        handle.cancel.cancel();
        // `handle` drops here → an online `Source` stops its own reconnect loop.
    }
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
                .source_infos()
                .into_iter()
                .map(|s| PersistedSource { id: s.id, url: s.url })
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
/// `rtsp://127.0.0.1:8554/device/{id}`). Best-effort: a short grace period, then
/// retry each program's `create()` with backing-off attempts over a ~1-minute
/// window so a program whose cameras are slow to come up still restores (its
/// still-offline sources then self-heal via `spawn_reconnecting`).
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
    tokio::time::sleep(RESTORE_GRACE).await;

    for pc in saved {
        let id = pc.id.clone();
        let mut backoff = RESTORE_RETRY_MIN;
        let mut elapsed = Duration::ZERO;
        loop {
            match create(pc.clone().into_params()).await {
                Ok(_) => {
                    log::info!("compositor restore: started '{id}'");
                    break;
                }
                Err(e) if elapsed < RESTORE_RETRY_BUDGET => {
                    log::warn!(
                        "compositor restore: '{id}' not ready yet ({e:#}); retry in {}s",
                        backoff.as_secs()
                    );
                    tokio::time::sleep(backoff).await;
                    elapsed += backoff;
                    backoff = (backoff * 2).min(RESTORE_RETRY_MAX);
                }
                Err(e) => {
                    log::error!(
                        "compositor restore: '{id}' gave up after ~{}s: {e:#}",
                        elapsed.as_secs()
                    );
                    break;
                }
            }
        }
    }
}
