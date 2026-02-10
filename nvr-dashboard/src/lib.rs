use axum::Router;
use axum_embed::ServeEmbed;
use rust_embed::RustEmbed;

#[derive(RustEmbed, Clone)]
#[folder = "app/dist/"]
struct AppAssets;

pub fn app_router(perfix: Option<&str>) -> Router {
    let serve_assets = ServeEmbed::<AppAssets>::new();

    let path = match perfix {
        Some(perfix) => perfix,
        None => "/",
    };

    Router::new().nest_service(path, serve_assets)
}
