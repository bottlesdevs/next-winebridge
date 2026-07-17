#![cfg_attr(windows, windows_subsystem = "windows")]

use bottles_core::proto::wine_bridge_server::WineBridgeServer;
use bottles_winebridge::WineBridgeService;
use std::{fs, io, net::SocketAddr, path::PathBuf};
use tokio::sync::oneshot;
use tonic::transport::server::TcpIncoming;
use tonic_health::server::health_reporter;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("bottles_winebridge=trace")),
        )
        .init();

    let port_file = PathBuf::from(std::env::var_os("WINEBRIDGE_PORT_FILE").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "WINEBRIDGE_PORT_FILE is not set",
        )
    })?);
    let incoming = TcpIncoming::bind(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    let addr = incoming.local_addr()?;

    let (tx, rx) = oneshot::channel();

    let service = WineBridgeService::new(tx);
    let (health_reporter, health_service) = health_reporter();
    health_reporter
        .set_serving::<WineBridgeServer<WineBridgeService>>()
        .await;
    let pending_port_file = port_file.with_extension("tmp");
    fs::write(&pending_port_file, addr.port().to_string())?;
    if let Err(error) = fs::rename(&pending_port_file, &port_file) {
        let _ = fs::remove_file(pending_port_file);
        return Err(error.into());
    }
    tracing::info!("WineBridge Agent listening on {}", addr);

    let result = tonic::transport::Server::builder()
        .add_service(health_service)
        .add_service(WineBridgeServer::new(service))
        .serve_with_incoming_shutdown(incoming, async move {
            let _ = rx.await;
            health_reporter
                .set_not_serving::<WineBridgeServer<WineBridgeService>>()
                .await;
            tracing::info!("Shutting down WineBridge Agent...");
        })
        .await;
    let _ = fs::remove_file(port_file);
    result?;
    Ok(())
}
