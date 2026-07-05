mod dll_overrides;
mod processes;
mod registry;
mod services;

use bottles_core::proto::{self as winebridge, wine_bridge_server::WineBridge};
use dll_overrides::manager::{DllOverrideManager, OverrideMode};
use processes::{manager::ProcessManager, process::ProcessIdentifier};
use registry::operations;
use services::manager::ServiceManager;
use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use tokio::sync::{Mutex, oneshot};
use tonic::{Request, Response, Result, Status};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetLogicalDrives, GetVolumeInformationW,
};
use windows::Win32::System::Threading::{CREATE_NEW_CONSOLE, CreateProcessW, STARTUPINFOW};
use windows::core::PCWSTR;

fn to_wide(s: &str) -> Vec<u16> {
    OsString::from(s).encode_wide().chain(Some(0)).collect()
}

pub struct WineBridgeService {
    shutdown_signal: Mutex<Option<oneshot::Sender<()>>>,
}

impl WineBridgeService {
    pub fn new(shutdown_signal: oneshot::Sender<()>) -> Self {
        Self {
            shutdown_signal: Mutex::new(Some(shutdown_signal)),
        }
    }
}

#[tonic::async_trait]
impl WineBridge for WineBridgeService {
    // --- Process Management ---

    async fn running_processes(
        &self,
        _request: Request<winebridge::RunningProcessesRequest>,
    ) -> Result<Response<winebridge::RunningProcessesResponse>> {
        let processes = ProcessManager
            .running_processes()
            .map_err(|e| Status::internal(format!("Failed to get running processes: {:?}", e)))?;

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
    ) -> Result<Response<winebridge::CreateProcessResponse>> {
        let pid = ProcessManager
            .execute(request.into_inner())
            .map_err(|e| Status::internal(format!("Failed to execute process: {:?}", e)))?;

        Ok(Response::new(winebridge::CreateProcessResponse { pid }))
    }

    async fn kill_process(
        &self,
        request: Request<winebridge::KillProcessRequest>,
    ) -> Result<Response<winebridge::KillProcessResponse>> {
        let pid = request.get_ref().pid;
        let process = ProcessManager
            .process(ProcessIdentifier::Pid(pid))
            .ok_or_else(|| Status::not_found("Process not found"))?;

        process
            .kill()
            .map_err(|e| Status::internal(format!("Failed to kill process: {:?}", e)))?;

        Ok(Response::new(winebridge::KillProcessResponse {
            success: true,
        }))
    }

    // --- Registry Management ---

    async fn create_registry_key(
        &self,
        request: Request<winebridge::RegistryKeyRequest>,
    ) -> Result<Response<()>> {
        let input = request.into_inner();
        operations::create_key(input.hive, &input.subkey)?;
        Ok(Response::new(()))
    }

    async fn delete_registry_tree(
        &self,
        request: Request<winebridge::RegistryKeyRequest>,
    ) -> Result<Response<()>> {
        let input = request.into_inner();
        operations::delete_tree(input.hive, &input.subkey)?;
        Ok(Response::new(()))
    }

    async fn get_registry_key(
        &self,
        request: Request<winebridge::RegistryKeyRequest>,
    ) -> Result<Response<winebridge::RegistryKey>> {
        let input = request.into_inner();
        Ok(Response::new(operations::get_key(
            input.hive,
            &input.subkey,
        )?))
    }

    async fn get_registry_value(
        &self,
        request: Request<winebridge::RegistryValueRequest>,
    ) -> Result<Response<winebridge::RegistryValue>> {
        let input = request.into_inner();
        Ok(Response::new(operations::get_value(
            input.hive,
            &input.subkey,
            &input.name,
        )?))
    }

    async fn set_registry_value(
        &self,
        request: Request<winebridge::SetRegistryValueRequest>,
    ) -> Result<Response<()>> {
        let input = request.into_inner();
        let value = input
            .value
            .and_then(|value| value.value)
            .ok_or_else(|| Status::invalid_argument("registry value is required"))?;
        operations::set_value(input.hive, &input.subkey, &input.name, value)?;
        Ok(Response::new(()))
    }

    async fn delete_registry_value(
        &self,
        request: Request<winebridge::RegistryValueRequest>,
    ) -> Result<Response<()>> {
        let input = request.into_inner();
        operations::delete_value(input.hive, &input.subkey, &input.name)?;
        Ok(Response::new(()))
    }

    // --- File System (New) ---

    async fn create_directory(
        &self,
        request: Request<winebridge::FileOperationRequest>,
    ) -> Result<Response<winebridge::FileOperationResponse>> {
        let path = request.into_inner().path;
        std::fs::create_dir_all(&path)
            .map(|_| winebridge::FileOperationResponse {
                success: true,
                error: String::new(),
            })
            .map_err(|e| Status::internal(e.to_string()))
            .map(Response::new)
    }

    async fn delete_file(
        &self,
        request: Request<winebridge::FileOperationRequest>,
    ) -> Result<Response<winebridge::FileOperationResponse>> {
        let path = request.into_inner().path;
        let p = Path::new(&path);
        let res = if p.is_dir() {
            std::fs::remove_dir_all(p)
        } else {
            std::fs::remove_file(p)
        };

        res.map(|_| winebridge::FileOperationResponse {
            success: true,
            error: String::new(),
        })
        .map_err(|e| Status::internal(e.to_string()))
        .map(Response::new)
    }

    async fn copy_file(
        &self,
        request: Request<winebridge::CopyMoveRequest>,
    ) -> Result<Response<winebridge::FileOperationResponse>> {
        let req = request.into_inner();
        // Simple copy, not recursive for dirs yet
        std::fs::copy(req.source, req.destination)
            .map(|_| winebridge::FileOperationResponse {
                success: true,
                error: String::new(),
            })
            .map_err(|e| Status::internal(e.to_string()))
            .map(Response::new)
    }

    async fn move_file(
        &self,
        request: Request<winebridge::CopyMoveRequest>,
    ) -> Result<Response<winebridge::FileOperationResponse>> {
        let req = request.into_inner();
        std::fs::rename(req.source, req.destination)
            .map(|_| winebridge::FileOperationResponse {
                success: true,
                error: String::new(),
            })
            .map_err(|e| Status::internal(e.to_string()))
            .map(Response::new)
    }

    async fn exists(
        &self,
        request: Request<winebridge::FileOperationRequest>,
    ) -> Result<Response<winebridge::ExistsResponse>> {
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
    ) -> Result<Response<winebridge::ListDirectoryResponse>> {
        let path = request.into_inner().path;
        let entries = std::fs::read_dir(path).map_err(|e| Status::internal(e.to_string()))?;

        let mut files = Vec::new();
        for entry in entries {
            if let Ok(entry) = entry
                && let Ok(meta) = entry.metadata()
            {
                files.push(winebridge::FileInfo {
                    name: entry.file_name().to_string_lossy().to_string(),
                    is_dir: meta.is_dir(),
                    size: meta.len(),
                });
            }
        }
        Ok(Response::new(winebridge::ListDirectoryResponse { files }))
    }

    // --- Service Management ---

    async fn list_services(
        &self,
        _request: Request<winebridge::ListServicesRequest>,
    ) -> Result<Response<winebridge::ListServicesResponse>> {
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
    ) -> Result<Response<winebridge::ServiceStatusResponse>> {
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
    ) -> Result<Response<winebridge::MessageResponse>> {
        let name = request.into_inner().name;
        ServiceManager
            .start(&name)
            .map(|_| {
                Response::new(winebridge::MessageResponse {
                    success: true,
                    error: None,
                })
            })
            .map_err(|e| Status::internal(format!("Failed to start service: {:?}", e)))
    }

    async fn stop_service(
        &self,
        request: Request<winebridge::ServiceRequest>,
    ) -> Result<Response<winebridge::MessageResponse>> {
        let name = request.into_inner().name;
        ServiceManager
            .stop(&name)
            .map(|_| {
                Response::new(winebridge::MessageResponse {
                    success: true,
                    error: None,
                })
            })
            .map_err(|e| Status::internal(format!("Failed to stop service: {:?}", e)))
    }

    async fn create_service(
        &self,
        request: Request<winebridge::CreateServiceRequest>,
    ) -> Result<Response<winebridge::MessageResponse>> {
        let input = request.into_inner();
        ServiceManager
            .create(
                &input.name,
                &input.display_name,
                &input.binary_path,
                input.start_type as u32,
            )
            .map(|_| {
                Response::new(winebridge::MessageResponse {
                    success: true,
                    error: None,
                })
            })
            .map_err(|e| Status::internal(format!("Failed to create service: {:?}", e)))
    }

    async fn delete_service(
        &self,
        request: Request<winebridge::ServiceRequest>,
    ) -> Result<Response<winebridge::MessageResponse>> {
        let name = request.into_inner().name;
        ServiceManager
            .delete(&name)
            .map(|_| {
                Response::new(winebridge::MessageResponse {
                    success: true,
                    error: None,
                })
            })
            .map_err(|e| Status::internal(format!("Failed to delete service: {:?}", e)))
    }

    // --- DLL Overrides ---

    async fn list_dll_overrides(
        &self,
        _request: Request<winebridge::ListDllOverridesRequest>,
    ) -> Result<Response<winebridge::ListDllOverridesResponse>> {
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

        Ok(Response::new(winebridge::ListDllOverridesResponse {
            overrides,
        }))
    }

    async fn get_dll_override(
        &self,
        request: Request<winebridge::DllOverrideRequest>,
    ) -> Result<Response<winebridge::DllOverrideResponse>> {
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
    ) -> Result<Response<winebridge::MessageResponse>> {
        let input = request.into_inner();
        let mode = OverrideMode::from_proto_i32(input.mode);
        DllOverrideManager
            .set(&input.dll, mode)
            .map(|_| {
                Response::new(winebridge::MessageResponse {
                    success: true,
                    error: None,
                })
            })
            .map_err(|e| Status::internal(format!("Failed to set DLL override: {:?}", e)))
    }

    async fn delete_dll_override(
        &self,
        request: Request<winebridge::DllOverrideRequest>,
    ) -> Result<Response<winebridge::MessageResponse>> {
        let dll = request.into_inner().dll;
        DllOverrideManager
            .delete(&dll)
            .map(|_| {
                Response::new(winebridge::MessageResponse {
                    success: true,
                    error: None,
                })
            })
            .map_err(|e| Status::internal(format!("Failed to delete DLL override: {:?}", e)))
    }

    // --- System ---

    async fn shutdown(
        &self,
        _request: Request<winebridge::ShutdownRequest>,
    ) -> Result<Response<winebridge::MessageResponse>> {
        if let Some(tx) = self.shutdown_signal.lock().await.take() {
            let _ = tx
                .send(())
                .map_err(|_| Status::internal("Failed to send shutdown signal"))?;
        }

        Ok(Response::new(winebridge::MessageResponse {
            success: true,
            error: None,
        }))
    }

    async fn wineboot(
        &self,
        request: Request<winebridge::WinebootRequest>,
    ) -> Result<Response<winebridge::MessageResponse>> {
        let mode = request.into_inner().mode;

        let args = match mode {
            1 => "/s",
            2 => "/k",
            _ => "/r",
        };

        let exe = to_wide("wineboot.exe");
        let mut cmd = to_wide(&format!("wineboot.exe {}", args));
        let startup_info = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
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
                &startup_info,
                &mut process_info,
            )
        };

        unsafe {
            CloseHandle(process_info.hProcess).ok();
            CloseHandle(process_info.hThread).ok();
        }

        result
            .map(|_| {
                Response::new(winebridge::MessageResponse {
                    success: true,
                    error: None,
                })
            })
            .map_err(|e| Status::internal(format!("Failed to execute wineboot: {:?}", e)))
    }

    async fn get_drive_info(
        &self,
        _request: Request<winebridge::DriveInfoRequest>,
    ) -> Result<Response<winebridge::DriveInfoResponse>> {
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
