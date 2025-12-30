use bottles_core::proto::winebridge::wine_bridge_server::WineBridgeServer;
use bottles_winebridge::WineBridgeService;
use tracing_subscriber::EnvFilter;
use tokio::sync::broadcast;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let addr = "[::1]:50051".parse().unwrap();
    let (tx, mut rx) = broadcast::channel(1);
    
    let service = WineBridgeService::new(tx);
    tracing::info!("WineBridge Agent listening on {}", addr);
    
    tonic::transport::Server::builder()
        .add_service(WineBridgeServer::new(service))
        .serve_with_shutdown(addr, async {
            rx.recv().await.ok();
            tracing::info!("Shutting down WineBridge Agent...");
        })
        .await?;
    Ok(())
}
