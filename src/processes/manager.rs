use super::process::{Process, ProcessIdentifier, ProcessInfo, ProcessSnapshot};
use bottles_core::proto as winebridge;
use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::PathBuf};
use windows::{
    Win32::System::Threading::{
        CREATE_NEW_CONSOLE, CreateProcessW, PROCESS_CREATION_FLAGS, STARTUPINFOW,
    },
    core::{Error, PCWSTR, PWSTR},
};

fn to_wide_string(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

pub struct ProcessManager;

impl ProcessManager {
    pub fn running_processes(&self) -> Result<Vec<Process>, Error> {
        let snapshot = ProcessSnapshot::new()?;

        Ok(snapshot.map(|process| process).collect())
    }

    pub fn process(&self, identifier: ProcessIdentifier) -> Option<Process> {
        let processes = self
            .running_processes()
            .expect("Failed to get running processes");

        match identifier {
            ProcessIdentifier::Name(name) => processes
                .iter()
                .find(|p| p.name().to_lowercase() == name.to_lowercase())
                .cloned(),
            ProcessIdentifier::Pid(pid) => processes.iter().find(|p| p.pid() == pid).cloned(),
        }
    }

    pub fn execute(&self, request: winebridge::CreateProcessRequest) -> Result<u32, Error> {
        let executable = PathBuf::from(request.command);
        let command_line = std::iter::once(executable.display().to_string())
            .chain(request.args)
            .collect::<Vec<_>>()
            .join(" ");

        let executable_w = to_wide_string(executable.as_os_str());
        let mut command_line = to_wide_string(command_line);
        let work_dir = request
            .work_dir
            .map(|work_dir| PCWSTR(to_wide_string(work_dir).as_ptr()))
            .unwrap_or_else(PCWSTR::null);

        let flags = if request.terminal {
            CREATE_NEW_CONSOLE
        } else {
            PROCESS_CREATION_FLAGS(0)
        };

        let mut startup_info = STARTUPINFOW::default();
        startup_info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
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
                &mut startup_info,
                &mut process_info.0,
            )?;
        }

        Ok(process_info.0.dwProcessId)
    }
}
