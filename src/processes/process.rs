use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::core::Error;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

pub enum ProcessIdentifier {
    Name(String),
    PID(u32),
}

#[derive(Debug, Clone)]
pub struct Process(PROCESSENTRY32W);

impl From<PROCESSENTRY32W> for Process {
    fn from(entry: PROCESSENTRY32W) -> Self {
        Self(entry)
    }
}

impl Process {
    pub fn name(&self) -> String {
        let len = self
            .0
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(self.0.szExeFile.len());

        OsString::from_wide(&self.0.szExeFile[..len])
            .to_string_lossy()
            .into_owned()
    }

    pub fn pid(&self) -> u32 {
        self.0.th32ProcessID
    }

    pub fn thread_count(&self) -> u32 {
        self.0.cntThreads
    }

    pub fn parent_pid(&self) -> u32 {
        self.0.th32ParentProcessID
    }

    pub fn priority_class(&self) -> i32 {
        self.0.pcPriClassBase
    }

    pub fn kill(&self) -> Result<(), Error> {
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, false, self.pid())? };

        unsafe { TerminateProcess(handle, 0) }
    }
}

pub struct ProcessSnapshot {
    handle: HANDLE,
    initialized: bool,
}

impl ProcessSnapshot {
    pub fn new() -> Result<Self, Error> {
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?;

        Ok(Self {
            handle: snapshot,
            initialized: false,
        })
    }
}

impl Iterator for ProcessSnapshot {
    type Item = Process;

    fn next(&mut self) -> Option<Self::Item> {
        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if !self.initialized {
            unsafe { Process32FirstW(self.handle, &mut entry) }.ok()?;
            self.initialized = true;
        } else {
            unsafe { Process32NextW(self.handle, &mut entry) }.ok()?;
        }

        Some(Process::from(entry))
    }
}

impl Drop for ProcessSnapshot {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle).ok() };
    }
}
