use proto::wine_bridge_server::{WineBridge, WineBridgeServer};
use windows::{
    core::{s, PCSTR},
    Win32::UI::WindowsAndMessaging::{MessageBoxA, MB_OK},
};

mod proto {
    tonic::include_proto!("winebridge");
}

#[derive(Debug, Default)]
struct WineBridgeService;

#[tonic::async_trait]
impl WineBridge for WineBridgeService {
    async fn message(
        &self,
        request: tonic::Request<proto::MessageRequest>,
    ) -> Result<tonic::Response<proto::MessageResponse>, tonic::Status> {
        let request = request.get_ref();
        println!("Got a request: {:?}", request);
        let message = request.message.as_str();
        let c_message = std::ffi::CString::new(message).unwrap();
        unsafe {
            MessageBoxA(
                None,
                PCSTR(c_message.as_ptr() as *const u8),
                s!("Hello"),
                MB_OK,
            );
        }
        Ok(tonic::Response::new(proto::MessageResponse {
            success: true,
        }))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse().unwrap();
    let service = WineBridgeService::default();
    println!("Listening on {}", addr);
    tonic::transport::Server::builder()
        .add_service(WineBridgeServer::new(service))
        .serve(addr)
        .await?;
    Ok(())
}
