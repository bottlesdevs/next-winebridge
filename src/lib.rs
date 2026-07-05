mod dll_overrides;
mod processes;
mod registry;
mod services;
mod status;

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

fn validated_path(value: &str) -> Result<&Path, Status> {
    if value.is_empty() || value.contains('\0') {
        Err(Status::invalid_argument(
            "path must be non-empty and contain no NUL bytes",
        ))
    } else {
        Ok(Path::new(value))
    }
}

fn path_info(path: &Path) -> Result<winebridge::PathInfo, Status> {
    let metadata = std::fs::metadata(path).map_err(status::io)?;
    let (kind, size) = if metadata.is_file() {
        (winebridge::PathKind::File, Some(metadata.len()))
    } else if metadata.is_dir() {
        (winebridge::PathKind::Directory, None)
    } else {
        return Err(Status::failed_precondition(
            "path is neither a regular file nor a directory",
        ));
    };

    Ok(winebridge::PathInfo {
        path: path.display().to_string(),
        kind: kind as i32,
        size,
    })
}

fn required(value: &str, field: &str) -> Result<(), Status> {
    if value.is_empty() || value.contains('\0') {
        Err(Status::invalid_argument(format!(
            "{field} must be non-empty and contain no NUL bytes"
        )))
    } else {
        Ok(())
    }
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

    async fn list_processes(
        &self,
        _request: Request<()>,
    ) -> Result<Response<winebridge::ListProcessesResponse>> {
        let processes = ProcessManager
            .running_processes()
            .map_err(status::windows)?;

        let processes = processes
            .iter()
            .map(|process| winebridge::Process {
                name: process.name(),
                pid: process.pid(),
                threads: process.thread_count(),
            })
            .collect();

        Ok(Response::new(winebridge::ListProcessesResponse {
            processes,
        }))
    }

    async fn launch_process(
        &self,
        request: Request<winebridge::LaunchProcessRequest>,
    ) -> Result<Response<winebridge::LaunchProcessResponse>> {
        let input = request.into_inner();
        if input.executable.is_empty()
            || input.executable.contains('\0')
            || input
                .arguments
                .iter()
                .any(|argument| argument.contains('\0'))
            || input
                .working_directory
                .as_deref()
                .is_some_and(|directory| directory.contains('\0'))
        {
            return Err(Status::invalid_argument(
                "process paths and arguments must be non-empty where required and contain no NUL bytes",
            ));
        }

        let pid = ProcessManager.execute(input).map_err(status::windows)?;

        Ok(Response::new(winebridge::LaunchProcessResponse { pid }))
    }

    async fn kill_process(
        &self,
        request: Request<winebridge::KillProcessRequest>,
    ) -> Result<Response<()>> {
        let pid = request.get_ref().pid;
        if pid == 0 {
            return Err(Status::invalid_argument("process id must be non-zero"));
        }
        let process = ProcessManager
            .process(ProcessIdentifier::Pid(pid))
            .map_err(status::windows)?
            .ok_or_else(|| Status::not_found("Process not found"))?;

        process.kill().map_err(status::windows)?;

        Ok(Response::new(()))
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
        request: Request<winebridge::PathRequest>,
    ) -> Result<Response<()>> {
        let path = request.into_inner().path;
        std::fs::create_dir_all(validated_path(&path)?).map_err(status::io)?;
        Ok(Response::new(()))
    }

    async fn delete_file(&self, request: Request<winebridge::PathRequest>) -> Result<Response<()>> {
        let path = request.into_inner().path;
        let path = validated_path(&path)?;
        if std::fs::metadata(path).map_err(status::io)?.is_dir() {
            return Err(Status::failed_precondition("path is a directory"));
        }
        std::fs::remove_file(path).map_err(status::io)?;
        Ok(Response::new(()))
    }

    async fn delete_directory_tree(
        &self,
        request: Request<winebridge::PathRequest>,
    ) -> Result<Response<()>> {
        let path = request.into_inner().path;
        let path = validated_path(&path)?;
        if !std::fs::metadata(path).map_err(status::io)?.is_dir() {
            return Err(Status::failed_precondition("path is not a directory"));
        }
        std::fs::remove_dir_all(path).map_err(status::io)?;
        Ok(Response::new(()))
    }

    async fn copy_file(
        &self,
        request: Request<winebridge::PathTransferRequest>,
    ) -> Result<Response<()>> {
        let req = request.into_inner();
        let source = validated_path(&req.source)?;
        if !std::fs::metadata(source).map_err(status::io)?.is_file() {
            return Err(Status::failed_precondition("source is not a file"));
        }
        std::fs::copy(source, validated_path(&req.destination)?).map_err(status::io)?;
        Ok(Response::new(()))
    }

    async fn move_path(
        &self,
        request: Request<winebridge::PathTransferRequest>,
    ) -> Result<Response<()>> {
        let req = request.into_inner();
        std::fs::rename(
            validated_path(&req.source)?,
            validated_path(&req.destination)?,
        )
        .map_err(status::io)?;
        Ok(Response::new(()))
    }

    async fn get_path_info(
        &self,
        request: Request<winebridge::PathRequest>,
    ) -> Result<Response<winebridge::PathInfo>> {
        let path = request.into_inner().path;
        Ok(Response::new(path_info(validated_path(&path)?)?))
    }

    async fn list_directory(
        &self,
        request: Request<winebridge::PathRequest>,
    ) -> Result<Response<winebridge::ListDirectoryResponse>> {
        let path = request.into_inner().path;
        let entries = std::fs::read_dir(validated_path(&path)?).map_err(status::io)?;
        let mut result = Vec::new();
        for entry in entries {
            result.push(path_info(&entry.map_err(status::io)?.path())?);
        }
        Ok(Response::new(winebridge::ListDirectoryResponse {
            entries: result,
        }))
    }

    // --- Service Management ---

    async fn list_services(
        &self,
        _request: Request<()>,
    ) -> Result<Response<winebridge::ListServicesResponse>> {
        let services = ServiceManager.list_services().map_err(status::windows)?;

        let services = services
            .into_iter()
            .map(services::to_proto)
            .collect::<Result<_, _>>()?;

        Ok(Response::new(winebridge::ListServicesResponse { services }))
    }

    async fn get_service(
        &self,
        request: Request<winebridge::ServiceRequest>,
    ) -> Result<Response<winebridge::Service>> {
        let name = request.into_inner().name;
        required(&name, "service name")?;

        Ok(Response::new(services::to_proto(
            ServiceManager.get(&name).map_err(status::windows)?,
        )?))
    }

    async fn start_service(
        &self,
        request: Request<winebridge::ServiceRequest>,
    ) -> Result<Response<()>> {
        let name = request.into_inner().name;
        required(&name, "service name")?;
        ServiceManager.start(&name).map_err(status::windows)?;
        Ok(Response::new(()))
    }

    async fn stop_service(
        &self,
        request: Request<winebridge::ServiceRequest>,
    ) -> Result<Response<()>> {
        let name = request.into_inner().name;
        required(&name, "service name")?;
        ServiceManager.stop(&name).map_err(status::windows)?;
        Ok(Response::new(()))
    }

    async fn create_service(
        &self,
        request: Request<winebridge::CreateServiceRequest>,
    ) -> Result<Response<()>> {
        let input = request.into_inner();
        required(&input.name, "service name")?;
        required(&input.binary_path, "service binary path")?;
        if input.display_name.contains('\0') {
            return Err(Status::invalid_argument(
                "service display name must contain no NUL bytes",
            ));
        }
        ServiceManager
            .create(
                &input.name,
                &input.display_name,
                &input.binary_path,
                services::start_type(input.start_type)?,
            )
            .map_err(status::windows)?;
        Ok(Response::new(()))
    }

    async fn delete_service(
        &self,
        request: Request<winebridge::ServiceRequest>,
    ) -> Result<Response<()>> {
        let name = request.into_inner().name;
        required(&name, "service name")?;
        ServiceManager.delete(&name).map_err(status::windows)?;
        Ok(Response::new(()))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_info_distinguishes_files_and_directories() {
        let directory = std::env::temp_dir().join(format!(
            "bottles-winebridge-path-info-{}",
            std::process::id()
        ));
        let file = directory.join("file.bin");
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&file, [1, 2, 3]).unwrap();

        let directory_info = path_info(&directory).unwrap();
        let file_info = path_info(&file).unwrap();
        assert_eq!(directory_info.kind(), winebridge::PathKind::Directory);
        assert_eq!(directory_info.size, None);
        assert_eq!(file_info.kind(), winebridge::PathKind::File);
        assert_eq!(file_info.size, Some(3));

        std::fs::remove_dir_all(directory).unwrap();
    }
}
