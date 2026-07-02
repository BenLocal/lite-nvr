//! Read-only API over the live GB registrar/catalog, for the dashboard picker.

use axum::{Router, extract::Path, routing::get};
use serde::Serialize;

use crate::handler::{ApiJsonResult, ok_json};

pub fn gb_router() -> Router {
    Router::new()
        .route("/devices", get(list_devices))
        .route("/catalog/{device_id}", get(catalog))
}

#[derive(Serialize)]
struct GbDeviceItem {
    device_id: String,
    online: bool,
}

#[derive(Serialize)]
struct GbChannelItem {
    channel_id: String,
    name: String,
    status: String,
}

/// Devices currently in the platform registrar. Empty when GB is disabled.
async fn list_devices() -> ApiJsonResult<Vec<GbDeviceItem>> {
    let Some(bridge) = crate::gb::bridge() else {
        return Ok(ok_json(Vec::new()));
    };
    let items = bridge
        .server()
        .devices()
        .into_iter()
        .map(|d| GbDeviceItem {
            device_id: d.device_id,
            online: d.online,
        })
        .collect();
    Ok(ok_json(items))
}

/// Query a device's channel catalog (live MANSCDP Catalog). Returns 503-ish
/// empty when GB is disabled; propagates crate errors otherwise.
async fn catalog(Path(device_id): Path<String>) -> ApiJsonResult<Vec<GbChannelItem>> {
    let Some(bridge) = crate::gb::bridge() else {
        return Ok(ok_json(Vec::new()));
    };
    let catalog = bridge.server().catalog_query(&device_id).await?;
    let items = catalog
        .items
        .into_iter()
        .map(|c| GbChannelItem {
            channel_id: c.device_id,
            name: c.name,
            status: c.status,
        })
        .collect();
    Ok(ok_json(items))
}
