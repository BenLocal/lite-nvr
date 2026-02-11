use axum::Router;
use axum_embed::{FallbackBehavior, ServeEmbed};
use rust_embed::RustEmbed;

#[derive(RustEmbed, Clone)]
#[folder = "app/dist/"]
struct AppAssets;

pub fn app_router(prefix: Option<&str>) -> Router {
    let serve_assets = ServeEmbed::<AppAssets>::with_parameters(
        Some("index.html".to_string()),
        FallbackBehavior::Ok,
        Some("index.html".to_string()),
    );

    let router = match prefix {
        Some(perfix) => Router::new().nest_service(perfix, serve_assets),
        None => Router::new().fallback_service(serve_assets),
    };

    router
}
