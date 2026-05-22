use bottles_core::proto::wine_bridge_server::WineBridgeServer;
use bottles_winebridge::WineBridgeService;
use tokio::sync::oneshot;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("bottles_winebridge=trace")),
        )
        .init();

    let host = std::env::var("WINEBRIDGE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port: u16 = std::env::var("WINEBRIDGE_PORT")
        .unwrap_or_else(|_| "50051".to_string())
        .parse()?;
    let addr = format!("{host}:{port}").parse().unwrap();

    let (tx, rx) = oneshot::channel();

    let service = WineBridgeService::new(tx);
    tracing::info!("WineBridge Agent listening on {}", addr);

    tonic::transport::Server::builder()
        .add_service(WineBridgeServer::new(service))
        .serve_with_shutdown(addr, async {
            let _ = rx.await;
            tracing::info!("Shutting down WineBridge Agent...");
        })
        .await?;
    Ok(())
}
