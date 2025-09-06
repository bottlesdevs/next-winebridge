use bottles_core::proto::wine_bridge_server::WineBridgeServer;
use bottles_winebridge::WineBridgeService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let addr = "[::1]:50051".parse().unwrap();
    let service = WineBridgeService::default();
    tracing::info!("Listening on {}", addr);
    tonic::transport::Server::builder()
        .add_service(WineBridgeServer::new(service))
        .serve(addr)
        .await?;
    Ok(())
}
