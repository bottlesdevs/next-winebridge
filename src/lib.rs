mod dll_overrides;
mod processes;
mod registry;
mod services;

use bottles_core::proto::winebridge::{self, wine_bridge_server::WineBridge};
use dll_overrides::manager::{DllOverrideManager, OverrideMode};
use processes::{manager::ProcessManager, process::ProcessIdentifier};
use registry::manager::{KeyExtension, RegistryManager, to_proto_reg_val, to_reg_data};
use services::manager::ServiceManager;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use tokio::sync::broadcast;
use tonic::{Request, Response, Status};
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetLogicalDrives, GetVolumeInformationW,
};
use windows::Win32::System::Threading::{CreateProcessW, CREATE_NEW_CONSOLE, STARTUPINFOW};
use windows::Win32::Foundation::CloseHandle;
use windows::core::PCWSTR;

fn to_wide(s: &str) -> Vec<u16> {
    OsString::from(s).encode_wide().chain(Some(0)).collect()
}

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
            error: None,
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
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
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
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
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
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
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
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
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
        let inner = request.into_inner();
        let path = Path::new(&inner.path);
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

    // --- Service Management ---

    async fn list_services(
        &self,
        _request: Request<winebridge::ListServicesRequest>,
    ) -> Result<Response<winebridge::ListServicesResponse>, Status> {
        let services = ServiceManager
            .list_services()
            .map_err(|e| Status::internal(format!("Failed to list services: {:?}", e)))?;

        let services = services
            .into_iter()
            .map(|s| winebridge::ServiceInfo {
                name: s.name,
                display_name: s.display_name,
                state: s.state as i32,
                start_type: s.start_type as i32,
            })
            .collect();

        Ok(Response::new(winebridge::ListServicesResponse { services }))
    }

    async fn get_service_status(
        &self,
        request: Request<winebridge::ServiceRequest>,
    ) -> Result<Response<winebridge::ServiceStatusResponse>, Status> {
        let name = request.into_inner().name;
        let state = ServiceManager
            .get_status(&name)
            .map_err(|e| Status::internal(format!("Failed to get service status: {:?}", e)))?;

        Ok(Response::new(winebridge::ServiceStatusResponse {
            name,
            state: state as i32,
        }))
    }

    async fn start_service(
        &self,
        request: Request<winebridge::ServiceRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let name = request.into_inner().name;
        ServiceManager
            .start(&name)
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
            .map_err(|e| Status::internal(format!("Failed to start service: {:?}", e)))
    }

    async fn stop_service(
        &self,
        request: Request<winebridge::ServiceRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let name = request.into_inner().name;
        ServiceManager
            .stop(&name)
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
            .map_err(|e| Status::internal(format!("Failed to stop service: {:?}", e)))
    }

    async fn create_service(
        &self,
        request: Request<winebridge::CreateServiceRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let input = request.into_inner();
        ServiceManager
            .create(&input.name, &input.display_name, &input.binary_path, input.start_type as u32)
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
            .map_err(|e| Status::internal(format!("Failed to create service: {:?}", e)))
    }

    async fn delete_service(
        &self,
        request: Request<winebridge::ServiceRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let name = request.into_inner().name;
        ServiceManager
            .delete(&name)
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
            .map_err(|e| Status::internal(format!("Failed to delete service: {:?}", e)))
    }

    // --- DLL Overrides ---

    async fn list_dll_overrides(
        &self,
        _request: Request<winebridge::ListDllOverridesRequest>,
    ) -> Result<Response<winebridge::ListDllOverridesResponse>, Status> {
        let overrides = DllOverrideManager
            .list()
            .map_err(|e| Status::internal(format!("Failed to list DLL overrides: {:?}", e)))?;

        let overrides = overrides
            .into_iter()
            .map(|o| winebridge::DllOverride {
                dll: o.dll,
                mode: o.mode.to_proto_i32(),
            })
            .collect();

        Ok(Response::new(winebridge::ListDllOverridesResponse { overrides }))
    }

    async fn get_dll_override(
        &self,
        request: Request<winebridge::DllOverrideRequest>,
    ) -> Result<Response<winebridge::DllOverrideResponse>, Status> {
        let dll = request.into_inner().dll;
        let entry = DllOverrideManager
            .get(&dll)
            .map_err(|e| Status::internal(format!("Failed to get DLL override: {:?}", e)))?;

        Ok(Response::new(winebridge::DllOverrideResponse {
            dll: entry.dll,
            mode: entry.mode.to_proto_i32(),
        }))
    }

    async fn set_dll_override(
        &self,
        request: Request<winebridge::SetDllOverrideRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let input = request.into_inner();
        let mode = OverrideMode::from_proto_i32(input.mode);
        DllOverrideManager
            .set(&input.dll, mode)
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
            .map_err(|e| Status::internal(format!("Failed to set DLL override: {:?}", e)))
    }

    async fn delete_dll_override(
        &self,
        request: Request<winebridge::DllOverrideRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let dll = request.into_inner().dll;
        DllOverrideManager
            .delete(&dll)
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
            .map_err(|e| Status::internal(format!("Failed to delete DLL override: {:?}", e)))
    }

    // --- System ---

    async fn shutdown(
        &self,
        _request: Request<winebridge::ShutdownRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let _ = self.shutdown_signal.send(());
        Ok(Response::new(winebridge::MessageResponse { success: true, error: None }))
    }

    async fn wineboot(
        &self,
        request: Request<winebridge::WinebootRequest>,
    ) -> Result<Response<winebridge::MessageResponse>, Status> {
        let mode = request.into_inner().mode;

        let args = match mode {
            1 => "/s",
            2 => "/k",
            _ => "/r",
        };

        let exe = to_wide("wineboot.exe");
        let mut cmd = to_wide(&format!("wineboot.exe {}", args));
        let mut startup_info = STARTUPINFOW::default();
        startup_info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut process_info = windows::Win32::System::Threading::PROCESS_INFORMATION::default();

        let result = unsafe {
            CreateProcessW(
                PCWSTR(exe.as_ptr()),
                Some(windows::core::PWSTR(cmd.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_NEW_CONSOLE,
                None,
                PCWSTR::null(),
                &mut startup_info,
                &mut process_info,
            )
        };

        unsafe {
            CloseHandle(process_info.hProcess).ok();
            CloseHandle(process_info.hThread).ok();
        }

        result
            .map(|_| Response::new(winebridge::MessageResponse { success: true, error: None }))
            .map_err(|e| Status::internal(format!("Failed to execute wineboot: {:?}", e)))
    }

    async fn get_drive_info(
        &self,
        _request: Request<winebridge::DriveInfoRequest>,
    ) -> Result<Response<winebridge::DriveInfoResponse>, Status> {
        let bitmask = unsafe { GetLogicalDrives() };
        let mut drives = Vec::new();

        for i in 0u32..26 {
            if bitmask & (1 << i) == 0 {
                continue;
            }

            let letter = (b'A' + i as u8) as char;
            let root = to_wide(&format!("{}:\\", letter));

            let mut label_buf = vec![0u16; 256];
            let mut fs_buf = vec![0u16; 256];

            unsafe {
                GetVolumeInformationW(
                    PCWSTR(root.as_ptr()),
                    Some(&mut label_buf),
                    None,
                    None,
                    None,
                    Some(&mut fs_buf),
                )
                .ok();
            }

            let label_len = label_buf.iter().position(|&c| c == 0).unwrap_or(0);
            let label = OsString::from_wide(&label_buf[..label_len])
                .to_string_lossy()
                .into_owned();

            let mut free_bytes: u64 = 0;
            let mut total_bytes: u64 = 0;

            unsafe {
                GetDiskFreeSpaceExW(
                    PCWSTR(root.as_ptr()),
                    Some(&mut free_bytes as *mut u64 as *mut _),
                    Some(&mut total_bytes as *mut u64 as *mut _),
                    None,
                )
                .ok();
            }

            drives.push(winebridge::Drive {
                letter: letter.to_string(),
                label,
                total_space: total_bytes,
                free_space: free_bytes,
            });
        }

        Ok(Response::new(winebridge::DriveInfoResponse { drives }))
    }
}
