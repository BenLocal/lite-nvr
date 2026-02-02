use axum::{Router, routing::get};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

pub(crate) fn start_api_server(cancel: CancellationToken) {
    tokio::spawn(async move {
        let app = Router::new().route("/", get(index));

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

async fn index() -> &'static str {
    "Hello, world!"
}
