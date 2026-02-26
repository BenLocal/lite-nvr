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

pub(crate) fn app_db_conn() -> anyhow::Result<turso::Connection> {
    get_app_db()?.connect()
}
