mod processes;
mod registry;

use bottles_core::proto::{self, wine_bridge_server::WineBridge};
use processes::{manager::ProcessManager, process::ProcessIdentifier};
use windows::{
    core::{s, PCSTR},
    Win32::UI::WindowsAndMessaging::{MessageBoxA, MB_OK},
};

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
}
