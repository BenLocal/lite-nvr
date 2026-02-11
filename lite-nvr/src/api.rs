use axum::Router;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub(crate) fn start_api_server(cancel: CancellationToken) {
    tokio::spawn(async move {
        let app = Router::new()
            .nest("/user", crate::handler::user::user_router())
            .nest("/pipe", crate::handler::media_pipe::meida_pipe_router())
            .nest("/system", crate::handler::system::system_router())
            .nest("/nvr", nvr_dashboard::app_router(None));

        let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
        println!("API server started on port 8080");
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal(cancel))
            .await
        {
            println!("Error starting API server: {}", e);
        }
    });
}

async fn shutdown_signal(cancel: CancellationToken) {
    tokio::select! {
        _ = cancel.cancelled() => {
            println!("Shutting down API server...");
        }
    }
}
