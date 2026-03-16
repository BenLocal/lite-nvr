use axum::{
    extract::Path,
    Json, Router,
    routing::{get, post},
};
use nvr_db::device::{Device, DeviceCreate, DeviceUpdate};

use crate::{
    db::app_db_conn,
    handler::ApiJsonResult,
};

pub fn device_router() -> Router {
    Router::new()
        .route("/", get(list_devices).post(create_device))
        .route("/{id}", post(update_device))
        .route("/{id}/delete", post(delete_device))
}

async fn list_devices() -> ApiJsonResult<Vec<Device>> {
    let conn = app_db_conn()?;
    let devices = nvr_db::device::query_all(&conn).await?;
    Ok(Json(devices))
}

async fn create_device(Json(req): Json<DeviceCreate>) -> ApiJsonResult<Device> {
    let conn = app_db_conn()?;
    let device = nvr_db::device::insert(&req, &conn).await?;
    Ok(Json(device))
}

async fn update_device(
    Path(id): Path<i64>,
    Json(req): Json<DeviceUpdate>,
) -> ApiJsonResult<Option<Device>> {
    let conn = app_db_conn()?;
    let device = nvr_db::device::update(id, &req, &conn).await?;
    Ok(Json(device))
}

async fn delete_device(Path(id): Path<i64>) -> ApiJsonResult<bool> {
    let conn = app_db_conn()?;
    let success = nvr_db::device::delete(id, &conn).await?;
    Ok(Json(success))
}
