mod processes;
mod registry;

use bottles_core::proto::{self, wine_bridge_server::WineBridge};
use processes::{manager::ProcessManager, process::ProcessIdentifier};
use registry::manager::{to_proto_reg_val, to_reg_data, KeyExtension, RegistryManager};
use windows::{
    core::{s, PCSTR},
    Win32::UI::WindowsAndMessaging::{MessageBoxA, MB_OK},
};
use windows_registry::Key;

#[derive(Debug, Default)]
pub struct WineBridgeService;

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

    async fn running_processes(
        &self,
        _request: tonic::Request<proto::RunningProcessesRequest>,
    ) -> Result<tonic::Response<proto::RunningProcessesResponse>, tonic::Status> {
        let processes = ProcessManager.running_processes().map_err(|e| {
            tonic::Status::internal(format!("Failed to get running processes: {:?}", e))
        })?;

        let processes = processes
            .iter()
            .map(|process| proto::Process {
                name: process.name(),
                pid: process.pid(),
                threads: process.thread_count(),
            })
            .collect();

        Ok(tonic::Response::new(proto::RunningProcessesResponse {
            processes,
        }))
    }

    async fn create_process(
        &self,
        request: tonic::Request<proto::CreateProcessRequest>,
    ) -> Result<tonic::Response<proto::CreateProcessResponse>, tonic::Status> {
        let input = request.into_inner();
        let program = std::path::Path::new(&input.command);
        let args = input.args;

        ProcessManager
            .execute(program, args)
            .map_err(|e| tonic::Status::internal(format!("Failed to execute process: {:?}", e)))?;

        Ok(tonic::Response::new(proto::CreateProcessResponse {
            pid: 0,
        }))
    }

    async fn kill_process(
        &self,
        request: tonic::Request<proto::KillProcessRequest>,
    ) -> Result<tonic::Response<proto::KillProcessResponse>, tonic::Status> {
        let pid = request.get_ref().pid;
        let process = ProcessManager
            .process(ProcessIdentifier::Pid(pid))
            .ok_or_else(|| tonic::Status::not_found("Process not found"))?;

        process
            .kill()
            .map_err(|e| tonic::Status::internal(format!("Failed to kill process: {:?}", e)))?;

        Ok(tonic::Response::new(proto::KillProcessResponse {}))
    }

    async fn create_registry_key(
        &self,
        request: tonic::Request<proto::CreateRegistryKeyRequest>,
    ) -> Result<tonic::Response<proto::MessageResponse>, tonic::Status> {
        let input = request.get_ref();
        let hive = input
            .hive
            .parse()
            .map_err(|e| tonic::Status::invalid_argument(format!("Invalid hive: {:?}", e)))?;
        let subkey = std::path::Path::new(input.subkey.as_str());

        RegistryManager
            .create_key(hive, subkey)
            .map(|_| tonic::Response::new(proto::MessageResponse { success: true }))
            .map_err(|e| tonic::Status::internal(format!("Failed to create registry key: {:?}", e)))
    }

    async fn delete_registry_key(
        &self,
        request: tonic::Request<proto::DeleteRegistryKeyRequest>,
    ) -> Result<tonic::Response<proto::MessageResponse>, tonic::Status> {
        let input = request.get_ref();
        let hive = input
            .hive
            .parse()
            .map_err(|e| tonic::Status::invalid_argument(format!("Invalid hive: {:?}", e)))?;
        let subkey = std::path::Path::new(input.subkey.as_str());

        RegistryManager
            .delete_key(hive, subkey)
            .map(|_| tonic::Response::new(proto::MessageResponse { success: true }))
            .map_err(|e| tonic::Status::internal(format!("Failed to create registry key: {:?}", e)))
    }

    async fn get_registry_key(
        &self,
        request: tonic::Request<proto::GetRegistryKeyRequest>,
    ) -> Result<tonic::Response<proto::RegistryKey>, tonic::Status> {
        let input = request.get_ref();
        let hive = input
            .hive
            .parse()
            .map_err(|e| tonic::Status::invalid_argument(format!("Invalid hive: {:?}", e)))?;
        let subkey = std::path::Path::new(input.subkey.as_str());

        let key = RegistryManager
            .key(hive, subkey)
            .map_err(|e| {
                tonic::Status::internal(format!("Failed to create registry key: {:?}", e))
            })?
            .as_registry_key(hive, subkey);

        Ok(tonic::Response::new(key))
    }

    async fn get_registry_key_value(
        &self,
        request: tonic::Request<proto::RegistryKeyRequest>,
    ) -> Result<tonic::Response<proto::RegistryValue>, tonic::Status> {
        let input = request.get_ref();
        let name = input.name.as_str();
        let hive = input
            .hive
            .parse()
            .map_err(|e| tonic::Status::invalid_argument(format!("Invalid hive: {:?}", e)))?;
        let subkey = std::path::Path::new(input.subkey.as_str());

        let key = RegistryManager.key(hive, subkey).map_err(|e| {
            tonic::Status::internal(format!("Failed to create registry key: {:?}", e))
        })?;

        let value = key.value(name).map_err(|e| {
            tonic::Status::internal(format!("Failed to get registry value: {:?}", e))
        })?;

        Ok(tonic::Response::new(to_proto_reg_val(value)))
    }

    async fn set_registry_key_value(
        &self,
        request: tonic::Request<proto::SetRegistryKeyValueRequest>,
    ) -> Result<tonic::Response<proto::MessageResponse>, tonic::Status> {
        let input = request.get_ref();

        let (name, key) = input
            .key
            .as_ref()
            .map(|k| {
                let hive = k.hive.parse().unwrap();
                let subkey = std::path::Path::new(k.subkey.as_str());
                (k.name.clone(), RegistryManager.key(hive, subkey).unwrap())
            })
            .ok_or_else(|| tonic::Status::invalid_argument("Key is required"))?;

        let value = input
            .value
            .as_ref()
            .ok_or_else(|| tonic::Status::invalid_argument("Value is required"))?;

        key.create_value(
            name.as_str(),
            to_reg_data(value.r#type(), value.data.clone()),
        )
        .map(|_| tonic::Response::new(proto::MessageResponse { success: true }))
        .map_err(|e| tonic::Status::internal(format!("Failed to set registry value: {:?}", e)))
    }

    async fn delete_registry_key_value(
        &self,
        request: tonic::Request<proto::RegistryKeyRequest>,
    ) -> Result<tonic::Response<proto::MessageResponse>, tonic::Status> {
        let input = request.get_ref();
        let name = input.name.as_str();
        let hive = input
            .hive
            .parse()
            .map_err(|e| tonic::Status::invalid_argument(format!("Invalid hive: {:?}", e)))?;
        let subkey = std::path::Path::new(input.subkey.as_str());

        RegistryManager
            .key(hive, subkey)
            .map_err(|e| {
                tonic::Status::internal(format!("Failed to create registry key: {:?}", e))
            })?
            .remove_value(name)
            .map(|_| tonic::Response::new(proto::MessageResponse { success: true }))
            .map_err(|e| {
                tonic::Status::internal(format!("Failed to delete registry value: {:?}", e))
            })
    }
}
