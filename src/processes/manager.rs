use super::process::{Process, ProcessIdentifier, ProcessInfo, ProcessSnapshot};
use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path};
use windows::{
    core::{Error, PCWSTR, PWSTR},
    Win32::System::Threading::{CreateProcessW, CREATE_NEW_CONSOLE, STARTUPINFOW},
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

    pub fn execute(&self, executable: &Path, args: Vec<String>) -> Result<(), Error> {
        let mut executable = to_wide_string(executable);
        let mut args = to_wide_string(args.join(" "));

        let mut startup_info = STARTUPINFOW::default();
        startup_info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        let mut process_info = ProcessInfo::default();

        unsafe {
            CreateProcessW(
                PCWSTR(executable.as_mut_ptr()),
                Some(PWSTR(args.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_NEW_CONSOLE,
                None,
                PCWSTR::null(),
                &mut startup_info,
                &mut process_info.0,
            )
        }
    }
}
