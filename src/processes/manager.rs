use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::PathBuf};

use super::process::{Process, ProcessInfo, ProcessSnapshot};
use bottles_core::proto as winebridge;
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            JobObjects::{AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject},
            Threading::{
                CREATE_NEW_CONSOLE, CREATE_SUSPENDED, CreateProcessW, PROCESS_CREATION_FLAGS,
                ResumeThread, STARTUPINFOW, TerminateProcess,
            },
        },
    },
    core::{Error, PCWSTR, PWSTR},
};

fn to_wide_string(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

struct Job(HANDLE);

impl Job {
    fn open(id: &str) -> Result<Self, Error> {
        let name = to_wide_string(id);
        Ok(Self(unsafe {
            CreateJobObjectW(None, PCWSTR(name.as_ptr()))?
        }))
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct ProcessManager;

impl ProcessManager {
    pub fn running_processes(&self) -> Result<Vec<Process>, Error> {
        Ok(ProcessSnapshot::new()?.collect())
    }

    pub fn execute(&self, request: winebridge::LaunchProcessRequest) -> Result<u32, Error> {
        let job = Job::open(&request.id)?;
        let executable = PathBuf::from(request.executable);
        let command_line = std::iter::once(executable.display().to_string())
            .chain(request.arguments)
            .collect::<Vec<_>>()
            .join(" ");

        let executable_w = to_wide_string(executable.as_os_str());
        let mut command_line = to_wide_string(command_line);
        // Keep the wide-encoded working directory alive for the whole CreateProcessW
        // call: the PCWSTR below borrows this buffer, so it must outlive the call.
        let work_dir_w = request.working_directory.map(to_wide_string);
        let work_dir = work_dir_w
            .as_ref()
            .map(|work_dir| PCWSTR(work_dir.as_ptr()))
            .unwrap_or_else(PCWSTR::null);
        let flags = CREATE_SUSPENDED
            | if request.new_console {
                CREATE_NEW_CONSOLE
            } else {
                PROCESS_CREATION_FLAGS(0)
            };
        let startup_info = STARTUPINFOW {
            cb: std::mem::size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut process_info = ProcessInfo::default();

        unsafe {
            CreateProcessW(
                PCWSTR(executable_w.as_ptr()),
                Some(PWSTR(command_line.as_mut_ptr())),
                None,
                None,
                false,
                flags,
                None,
                work_dir,
                &startup_info,
                &mut process_info.0,
            )?;

            if let Err(error) = AssignProcessToJobObject(job.0, process_info.0.hProcess) {
                let _ = TerminateProcess(process_info.0.hProcess, 1);
                return Err(error);
            }
            if ResumeThread(process_info.0.hThread) == u32::MAX {
                let error = Error::from_thread();
                let _ = TerminateProcess(process_info.0.hProcess, 1);
                return Err(error);
            }
        }

        Ok(process_info.0.dwProcessId)
    }

    pub fn kill(&self, id: &str) -> Result<(), Error> {
        let job = Job::open(id)?;
        unsafe { TerminateJobObject(job.0, 0) }
    }
}
