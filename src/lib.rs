mod processes;
mod registry;

use bottles_core::proto::winebridge::{self, wine_bridge_server::WineBridge};
use processes::{manager::ProcessManager, process::ProcessIdentifier};
use registry::manager::{KeyExtension, RegistryManager, to_proto_reg_val, to_reg_data};
use std::path::Path;
use tokio::sync::broadcast;
use tonic::{Request, Response, Status};

pub struct WineBridgeService {
    shutdown_signal: broadcast::Sender<()>,
}

impl WineBridgeService {
    pub fn new(shutdown_signal: broadcast::Sender<()>) -> Self {
        Self { shutdown_signal }
    }
}

#[tonic::async_trait]
impl WineBridge for WineBridgeService {
    async fn message(
        &self,
        request: Request<winebridge::MessageRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let _ = request;
        Ok(Response::new(winebridge::MessageResponse {
            success: true,
            error: String::new(),
        }))
    }

    // --- Process Management ---

    async fn running_processes(
        &self,
        _request: Request<winebridge::RunningProcessesRequest>,
    ) -> Result<Response<winebridge::RunningProcessesResponse>, Status> {
        let processes = ProcessManager.running_processes().map_err(|e| {
            Status::internal(format!("Failed to get running processes: {:?}", e))
        })?;

        let processes = processes
            .iter()
            .map(|process| winebridge::Process {
                name: process.name(),
                pid: process.pid(),
                threads: process.thread_count(),
            })
            .collect();

        Ok(Response::new(winebridge::RunningProcessesResponse {
            processes,
        }))
    }

    async fn create_process(
        &self,
        request: Request<winebridge::CreateProcessRequest>,
    ) -> Result<Response<winebridge::CreateProcessResponse>, Status> {
        let input = request.into_inner();
        let program = std::path::Path::new(&input.command);
        let args = input.args;
        
        // TODO: Handle work_dir and env from input
        
        ProcessManager
            .execute(program, args)
            .map_err(|e| Status::internal(format!("Failed to execute process: {:?}", e)))?;

        Ok(Response::new(winebridge::CreateProcessResponse {
            pid: 0, // TODO: Return real PID
        }))
    }

    async fn kill_process(
        &self,
        request: Request<winebridge::KillProcessRequest>,
    ) -> Result<Response<winebridge::KillProcessResponse>, Status> {
        let pid = request.get_ref().pid;
        let process = ProcessManager
            .process(ProcessIdentifier::Pid(pid))
            .ok_or_else(|| Status::not_found("Process not found"))?;

        process
            .kill()
            .map_err(|e| Status::internal(format!("Failed to kill process: {:?}", e)))?;

        Ok(Response::new(winebridge::KillProcessResponse { success: true }))
    }

    // --- Registry Management ---

    async fn create_registry_key(
        &self,
        request: Request<winebridge::CreateRegistryKeyRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let input = request.get_ref();
        let hive = input.hive.parse().map_err(|e| Status::invalid_argument(format!("{:?}", e)))?;
        let subkey = Path::new(&input.subkey);

        RegistryManager.create_key(hive, subkey)
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: String::new() }))
            .map_err(|e| Status::internal(format!("{:?}", e)))
    }

    async fn delete_registry_key(
        &self,
        request: Request<winebridge::DeleteRegistryKeyRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let input = request.get_ref();
        let hive = input.hive.parse().map_err(|e| Status::invalid_argument(format!("{:?}", e)))?;
        let subkey = Path::new(&input.subkey);

        RegistryManager.delete_key(hive, subkey)
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: String::new() }))
            .map_err(|e| Status::internal(format!("{:?}", e)))
    }

    async fn get_registry_key(
        &self,
        request: Request<winebridge::GetRegistryKeyRequest>,
    ) -> Result<Response<winebridge::RegistryKey>, Status> {
        let input = request.get_ref();
        let hive = input.hive.parse().map_err(|e| Status::invalid_argument(format!("{:?}", e)))?;
        let subkey = Path::new(&input.subkey);

        let key = RegistryManager.key(hive, subkey)
            .map_err(|e| Status::internal(format!("{:?}", e)))?
            .as_registry_key(hive, subkey);

        Ok(Response::new(key))
    }

    async fn get_registry_key_value(
        &self,
        request: Request<winebridge::RegistryKeyRequest>,
    ) -> Result<Response<winebridge::RegistryValue>, Status> {
        let input = request.get_ref();
        let hive = input.hive.parse().map_err(|e| Status::invalid_argument(format!("{:?}", e)))?;
        let subkey = Path::new(&input.subkey);

        let key = RegistryManager.key(hive, subkey)
            .map_err(|e| Status::internal(format!("{:?}", e)))?;
            
        let value = key.value(&input.name)
            .map_err(|e| Status::internal(format!("{:?}", e)))?;

        Ok(Response::new(to_proto_reg_val(value)))
    }

    async fn set_registry_key_value(
        &self,
        request: Request<winebridge::SetRegistryKeyValueRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let input = request.get_ref();
        let key_req = input.key.as_ref().ok_or(Status::invalid_argument("Missing key"))?;
        let hive = key_req.hive.parse().map_err(|e| Status::invalid_argument(format!("{:?}", e)))?;
        let subkey = Path::new(&key_req.subkey);
        
        let key = RegistryManager.key(hive, subkey)
            .map_err(|e| Status::internal(format!("{:?}", e)))?;

        let value = input.value.as_ref().ok_or(Status::invalid_argument("Missing value"))?;
        
        key.create_value(&key_req.name, to_reg_data(value.r#type(), value.data.clone()))
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: String::new() }))
            .map_err(|e| Status::internal(format!("{:?}", e)))
    }

    async fn delete_registry_key_value(
        &self,
        request: Request<winebridge::RegistryKeyRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let input = request.get_ref();
        let hive = input.hive.parse().map_err(|e| Status::invalid_argument(format!("{:?}", e)))?;
        let subkey = Path::new(&input.subkey);

        RegistryManager.key(hive, subkey)
            .map_err(|e| Status::internal(format!("{:?}", e)))?
            .remove_value(&input.name)
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: String::new() }))
            .map_err(|e| Status::internal(format!("{:?}", e)))
    }

    // --- File System (New) ---

    async fn create_directory(
        &self,
        request: Request<winebridge::FileOperationRequest>,
    ) -> Result<Response<winebridge::FileOperationResponse>, Status> {
        let path = request.into_inner().path;
        std::fs::create_dir_all(&path)
            .map(|_| winebridge::FileOperationResponse { success: true, error: String::new() })
            .map_err(|e| Status::internal(e.to_string()))
            .map(Response::new)
    }

    async fn delete_file(
        &self,
        request: Request<winebridge::FileOperationRequest>,
    ) -> Result<Response<winebridge::FileOperationResponse>, Status> {
        let path = request.into_inner().path;
        let p = Path::new(&path);
        let res = if p.is_dir() {
            std::fs::remove_dir_all(p)
        } else {
            std::fs::remove_file(p)
        };

        res.map(|_| winebridge::FileOperationResponse { success: true, error: String::new() })
           .map_err(|e| Status::internal(e.to_string()))
           .map(Response::new)
    }

    async fn copy_file(
        &self,
        request: Request<winebridge::CopyMoveRequest>,
    ) -> Result<Response<winebridge::FileOperationResponse>, Status> {
        let req = request.into_inner();
        // Simple copy, not recursive for dirs yet
        std::fs::copy(req.source, req.destination)
            .map(|_| winebridge::FileOperationResponse { success: true, error: String::new() })
            .map_err(|e| Status::internal(e.to_string()))
            .map(Response::new)
    }

    async fn move_file(
        &self,
        request: Request<winebridge::CopyMoveRequest>,
    ) -> Result<Response<winebridge::FileOperationResponse>, Status> {
        let req = request.into_inner();
        std::fs::rename(req.source, req.destination)
            .map(|_| winebridge::FileOperationResponse { success: true, error: String::new() })
            .map_err(|e| Status::internal(e.to_string()))
            .map(Response::new)
    }

    async fn exists(
        &self,
        request: Request<winebridge::FileOperationRequest>,
    ) -> Result<Response<winebridge::ExistsResponse>, Status> {
        let path = Path::new(&request.into_inner().path);
        Ok(Response::new(winebridge::ExistsResponse {
            exists: path.exists(),
            is_dir: path.is_dir(),
        }))
    }

    async fn list_directory(
        &self,
        request: Request<winebridge::FileOperationRequest>,
    ) -> Result<Response<winebridge::ListDirectoryResponse>, Status> {
        let path = request.into_inner().path;
        let entries = std::fs::read_dir(path).map_err(|e| Status::internal(e.to_string()))?;
        
        let mut files = Vec::new();
        for entry in entries {
            if let Ok(entry) = entry {
                if let Ok(meta) = entry.metadata() {
                    files.push(winebridge::FileInfo {
                        name: entry.file_name().to_string_lossy().to_string(),
                        is_dir: meta.is_dir(),
                        size: meta.len(),
                    });
                }
            }
        }
        Ok(Response::new(winebridge::ListDirectoryResponse { files }))
    }

    // --- System ---

    async fn shutdown(
        &self,
        _request: Request<winebridge::ShutdownRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let _ = self.shutdown_signal.send(());
        Ok(Response::new(winebridge::MessageResponse { success: true, error: String::new() }))
    }

    async fn wineboot(
        &self,
        request: Request<winebridge::WinebootRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        // Mock implementation relying on external wineboot or just killing processes
        // In a real scenario, this would execute 'wineboot.exe'
        let _req = request.into_inner();
        // TODO: Execute wineboot
        Ok(Response::new(winebridge::MessageResponse { success: true, error: String::new() }))
    }

    async fn get_drive_info(
        &self,
        _request: Request<winebridge::DriveInfoRequest>,
    ) -> Result<Response<winebridge::DriveInfoResponse>, Status> {
         // Mock implementation
         let drives = vec![
             winebridge::Drive { letter: "C".to_string(), label: "System".to_string(), total_space: 1000, free_space: 500 },
             winebridge::Drive { letter: "Z".to_string(), label: "Root".to_string(), total_space: 2000, free_space: 1000 },
         ];
         Ok(Response::new(winebridge::DriveInfoResponse { drives }))
    }
}
