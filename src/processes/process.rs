use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::core::Error;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    OpenProcess, TerminateProcess, PROCESS_INFORMATION, PROCESS_TERMINATE,
};

pub enum ProcessIdentifier {
    Name(String),
    PID(u32),
}

#[derive(Default)]
pub struct ProcessInfo(pub PROCESS_INFORMATION);

impl Drop for ProcessInfo {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0.hProcess).ok();
            CloseHandle(self.0.hThread).ok();
        }
    }
}

pub enum PriorityClass {
    Idle = 0x00000040,
    BelowNormal = 0x00004000,
    Normal = 0x00000020,
    AboveNormal = 0x00008000,
    High = 0x00000080,
    Realtime = 0x00000100,
}

impl PriorityClass {
    fn from(value: i32) -> Self {
        match value {
            0x00000040 => PriorityClass::Idle,
            0x00004000 => PriorityClass::BelowNormal,
            0x00000020 => PriorityClass::Normal,
            0x00008000 => PriorityClass::AboveNormal,
            0x00000080 => PriorityClass::High,
            0x00000100 => PriorityClass::Realtime,
            _ => panic!("Invalid priority class"),
        }
    }
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

    pub fn priority_class(&self) -> PriorityClass {
        PriorityClass::from(self.0.pcPriClassBase)
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
