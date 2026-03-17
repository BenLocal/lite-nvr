use axum::Router;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub(crate) fn start_api_server(cancel: CancellationToken, port: u16) {
    tokio::spawn(async move {
        let api = Router::new()
            .nest("/device", crate::handler::device::device_router())
            .nest("/user", crate::handler::user::user_router())
            .nest("/pipe", crate::handler::media_pipe::media_pipe_router())
            .nest("/system", crate::handler::system::system_router());

        let app = Router::new()
            .nest("/api", api)
            .nest("/nvr", nvr_dashboard::app_router(None));

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
