use axum::Router;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub(crate) fn start_api_server(cancel: CancellationToken, port: u16) {
    tokio::spawn(async move {
        let api = Router::new()
            .nest("/device", crate::handler::device::device_router())
            .nest("/playback", crate::handler::playback::playback_router())
            .nest("/user", crate::handler::user::user_router())
            .nest("/pipe", crate::handler::media_pipe::media_pipe_router())
            .nest("/system", crate::handler::system::system_router())
            .nest("/gb", crate::gb::api::gb_router())
            .nest("/transport", crate::transport::api::transport_router())
            .nest("/program", crate::program::api::program_router())
            .nest("/compositor", crate::compositor::api::compositor_router())
            .nest("/audiomixer", crate::audiomixer::api::audiomixer_router())
            .nest("/asr", crate::asr::api::asr_router());

        let (asr_layer, asr_io) = crate::asr::build_socketio();

        let app = Router::new()
            .nest("/api", api)
            // Mount the dashboard via its prefix-aware branch (nest_service), which
            // serves the bare SPA root `/nvr/`. Nesting the fallback-based
            // `app_router(None)` under `/nvr` instead makes axum 404 `/nvr/`.
            .merge(nvr_dashboard::app_router(Some("/nvr")))
            // Reverse-proxy `/media/*` to ZLM's HTTP service (HTTP + WS).
            .merge(crate::proxy::media_proxy_router())
            // Socket.IO `/asr` namespace for live transcripts.
            .layer(asr_layer);

        crate::asr::hub::AsrHub::init(asr_io, crate::asr::model_config());

        let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
            .await
            .unwrap();
        log::info!("API server started on port {}", port);
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal(cancel))
            .await
        {
            log::error!("Error starting API server: {}", e);
        }
    });
}

async fn shutdown_signal(cancel: CancellationToken) {
    tokio::select! {
        _ = cancel.cancelled() => {
            log::info!("Shutting down API server...");
        }
    }
}
