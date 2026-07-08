use std::sync::OnceLock;

use nvr_db::db::{DatabaseConfig, NvrDatabase};

static APP_DB: OnceLock<NvrDatabase> = OnceLock::new();

pub(crate) async fn init_app_db(url: &str) -> anyhow::Result<&'static NvrDatabase> {
    let config = DatabaseConfig::new(url);
    let db = NvrDatabase::new(&config)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to init app db: {:?}", e))?;
    APP_DB
        .set(db)
        .map_err(|_| anyhow::anyhow!("Failed to set APP_DB"))?;
    Ok(APP_DB.get().unwrap())
}

fn get_app_db() -> anyhow::Result<&'static NvrDatabase> {
    Ok(APP_DB
        .get()
        .ok_or(anyhow::anyhow!("APP_DB not initialized"))?)
}

/// Returns a fresh `turso::Connection` for the current unit of work.
///
/// This deliberately opens a new connection per call rather than caching and
/// cloning a single shared one. Investigation of `turso` 0.6.1 (the locked
/// version) showed that reuse-by-sharing is unsafe here:
///
/// * `turso::Connection` is `Clone + Send + Sync` and a clone is a cheap
///   `Arc` bump of the same underlying `TursoConnection`, so it is *tempting*
///   to cache one connection and hand out clones.
/// * But every clone shares one `Arc<ConcurrentGuard>`, and each `query`/
///   `execute`/`step` first calls `ConcurrentGuard::try_use()`, a non-blocking
///   `compare_exchange` that returns `Misuse("concurrent use forbidden")`
///   (rather than waiting) when the connection is already in use. On our
///   multi-threaded runtime (`#[tokio::main]` default) two overlapping
///   requests driving the shared connection therefore hit hard, intermittent
///   errors. An empirical multi-thread test confirmed this: a shared clone
///   produced thousands of `concurrent use forbidden` errors under
///   concurrency, whereas the per-connection pattern below produced none of
///   them (only the normal, pre-existing `database is locked` WAL
///   write-contention that is independent of this choice).
/// * The signature is fixed at `-> anyhow::Result<turso::Connection>` (all
///   callers own the returned value), so there is no check-in hook on which to
///   build an exclusive-checkout pool; any pool would have to share
///   connections and reintroduce the hazard above.
/// * turso's high-level `query`/`execute` use `prepare_single` (a fresh parse
///   each call), not the cached-statement path, so caching a connection would
///   not even warm a statement cache for these callers — the only saving would
///   be one lightweight `turso_core` connection allocation, not worth the
///   correctness risk.
///
/// Opening one connection per request is turso's intended concurrency pattern
/// (each concurrently-executing unit of work gets its own, exclusively-used
/// connection) and is correct under WAL. If a future turso version exposes a
/// blocking/pooled connection or a scoped checkout guard, revisit this.
pub(crate) fn app_db_conn() -> anyhow::Result<turso::Connection> {
    get_app_db()?.connect()
}
