use tokio_util::sync::CancellationToken;

mod api;

#[tokio::main]
async fn main() -> ! {
    let cancel = CancellationToken::new();

    let cancel_clone = cancel.clone();
    api::start_api_server(cancel_clone);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                break;
            },
            _ = tokio::signal::ctrl_c() => {
                cancel.cancel();
            },
        }
    }

    std::process::exit(0);
}
