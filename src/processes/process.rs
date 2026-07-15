use std::{ffi::OsString, os::windows::ffi::OsStringExt};

use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::PROCESS_INFORMATION,
        },
    },
    core::Error,
};

#[derive(Default)]
pub struct ProcessInfo(pub PROCESS_INFORMATION);

impl Drop for ProcessInfo {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0.hProcess);
            let _ = CloseHandle(self.0.hThread);
        }
    }
}

#[derive(Debug, Clone)]
pub struct Process(PROCESSENTRY32W);

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
}

pub struct ProcessSnapshot {
    handle: HANDLE,
    initialized: bool,
}

impl ProcessSnapshot {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            handle: unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }?,
            initialized: false,
        })
    }
}

impl Iterator for ProcessSnapshot {
    type Item = Process;

    fn next(&mut self) -> Option<Self::Item> {
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };

        if !self.initialized {
            unsafe { Process32FirstW(self.handle, &mut entry) }.ok()?;
            self.initialized = true;
        } else {
            unsafe { Process32NextW(self.handle, &mut entry) }.ok()?;
        }

        Some(Process(entry))
    }
}

impl Drop for ProcessSnapshot {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
