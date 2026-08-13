#![cfg_attr(windows, windows_subsystem = "windows")]

use bottles_winebridge::WineBridgeService;
use next_proto::winebridge::wine_bridge_server::WineBridgeServer;
use std::{fs, io, net::SocketAddr, path::PathBuf};
use tokio::sync::oneshot;
use tonic_health::server::health_reporter;
use tracing_subscriber::EnvFilter;

mod transport;
use transport::Incoming;

/// Set by next-core when the runner's Wine cannot poll sockets, so `main`
/// binds the blocking transport instead of the normal tokio one.
const BLOCKING_TRANSPORT_ENV: &str = "WINEBRIDGE_BLOCKING_TRANSPORT";

#[cfg(windows)]
fn acquire_instance()
-> windows::core::Result<Option<windows::core::Owned<windows::Win32::Foundation::HANDLE>>> {
    use windows::{
        Win32::{
            Foundation::{ERROR_ALREADY_EXISTS, GetLastError},
            System::Threading::CreateMutexW,
        },
        core::{Owned, w},
    };

    let handle = unsafe { CreateMutexW(None, true, w!("BottlesNextWineBridge"))? };
    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let handle = unsafe { Owned::new(handle) };
    Ok((!already_exists).then_some(handle))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("bottles_winebridge=trace")),
        )
        .init();

    #[cfg(windows)]
    let Some(_instance) = acquire_instance()? else {
        return Ok(());
    };

    let port_file = PathBuf::from(std::env::var_os("WINEBRIDGE_PORT_FILE").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "WINEBRIDGE_PORT_FILE is not set",
        )
    })?);
    let blocking_transport = std::env::var_os(BLOCKING_TRANSPORT_ENV).is_some();
    let incoming = Incoming::bind(SocketAddr::from(([127, 0, 0, 1], 0)), blocking_transport)?;
    let addr = incoming.local_addr()?;
    if blocking_transport {
        tracing::info!("Using the blocking transport (host Wine cannot poll sockets)");
    }

    let (tx, rx) = oneshot::channel();

    let service = WineBridgeService::new(tx);
    let (health_reporter, health_service) = health_reporter();
    health_reporter
        .set_serving::<WineBridgeServer<WineBridgeService>>()
        .await;
    if let Some(parent) = port_file.parent() {
        fs::create_dir_all(parent)?;
    }
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
