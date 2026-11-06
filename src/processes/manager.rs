use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, c_void},
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use super::process::{ProcessInfo, ProcessSnapshot};
use next_proto::winebridge;
use tokio::sync::watch;
use windows::{
    Win32::{
        Foundation::{
            CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, HANDLE, INVALID_HANDLE_VALUE,
        },
        System::{
            IO::{CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED},
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOBOBJECT_ASSOCIATE_COMPLETION_PORT,
                JOBOBJECT_BASIC_PROCESS_ID_LIST, JobObjectAssociateCompletionPortInformation,
                JobObjectBasicProcessIdList, OpenJobObjectW, QueryInformationJobObject,
                SetInformationJobObject, TerminateJobObject,
            },
            SystemServices::{JOB_OBJECT_QUERY, JOB_OBJECT_TERMINATE},
            Threading::{
                CREATE_NEW_CONSOLE, CREATE_SUSPENDED, CreateProcessW, INFINITE,
                PROCESS_CREATION_FLAGS, ResumeThread, STARTUPINFOW, TerminateProcess,
            },
        },
    },
    core::{Error, PCWSTR, PWSTR},
};

fn to_wide_string(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

/// A process group: the processes launched with one group ID and their children.
///
/// Backed by a job object named after the group ID, so the group outlives
/// WineBridge while any of its processes run.
struct Group(HANDLE);

// Job handles are kernel handles and may be used from any thread.
unsafe impl Send for Group {}
unsafe impl Sync for Group {}

impl Group {
    /// Creates group `id`, or opens it if it already exists.
    fn create(id: &str) -> Result<Self, Error> {
        let name = to_wide_string(id);
        Ok(Self(unsafe {
            CreateJobObjectW(None, PCWSTR(name.as_ptr()))?
        }))
    }

    /// Opens group `id`, or returns `None` if none of its processes is running.
    fn open(id: &str) -> Result<Option<Self>, Error> {
        let name = to_wide_string(id);
        let access = JOB_OBJECT_QUERY | JOB_OBJECT_TERMINATE;
        match unsafe { OpenJobObjectW(access, false, PCWSTR(name.as_ptr())) } {
            Ok(handle) => Ok(Some(Self(handle))),
            Err(error) if error.code() == ERROR_FILE_NOT_FOUND.to_hresult() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Returns the IDs of the group's running processes.
    fn pids(&self) -> Result<HashSet<u32>, Error> {
        let mut count: usize = 1;
        loop {
            // The struct already holds one ID; each further ID adds one `usize`.
            let bytes = size_of::<JOBOBJECT_BASIC_PROCESS_ID_LIST>()
                + count.saturating_sub(1) * size_of::<usize>();
            // A `usize` buffer keeps the list aligned.
            let mut buffer = vec![0usize; bytes.div_ceil(size_of::<usize>())];
            let result = unsafe {
                QueryInformationJobObject(
                    Some(self.0),
                    JobObjectBasicProcessIdList,
                    buffer.as_mut_ptr() as *mut c_void,
                    bytes as u32,
                    None,
                )
            };
            let list = unsafe { &*(buffer.as_ptr() as *const JOBOBJECT_BASIC_PROCESS_ID_LIST) };
            match result {
                Ok(()) => {
                    let ids = unsafe {
                        std::slice::from_raw_parts(
                            list.ProcessIdList.as_ptr(),
                            list.NumberOfProcessIdsInList as usize,
                        )
                    };
                    return Ok(ids.iter().map(|&id| id as u32).collect());
                }
                Err(error) if error.code() == ERROR_MORE_DATA.to_hresult() => {
                    count = list.NumberOfAssignedProcesses as usize;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn terminate(&self) -> Result<(), Error> {
        unsafe { TerminateJobObject(self.0, 0) }
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Receives a notification whenever a process in a launched group starts or exits.
struct CompletionPort(HANDLE);

// The port handle is only used to wait for events on the watcher thread.
unsafe impl Send for CompletionPort {}
unsafe impl Sync for CompletionPort {}

/// Launches processes into groups and publishes which groups are running.
///
/// Every group launched here reports to one completion port. A watcher thread
/// republishes the running groups whenever a process in any of them starts or
/// exits. Process details are read on request rather than published.
pub struct ProcessManager {
    port: CompletionPort,
    /// Groups launched by this WineBridge, by group ID.
    groups: Mutex<HashMap<String, Group>>,
    running: watch::Sender<winebridge::RunningGroups>,
}

impl ProcessManager {
    /// Creates the completion port and starts the thread that watches it.
    pub fn new() -> Result<Arc<Self>, Error> {
        let port = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, 1)? };
        let manager = Arc::new(Self {
            port: CompletionPort(port),
            groups: Mutex::new(HashMap::new()),
            running: watch::channel(winebridge::RunningGroups::default()).0,
        });
        let watcher = manager.clone();
        std::thread::spawn(move || watcher.watch());
        Ok(manager)
    }

    /// Subscribes to the running groups, starting with the current ones.
    pub fn subscribe(&self) -> watch::Receiver<winebridge::RunningGroups> {
        self.running.subscribe()
    }

    /// Lists the processes in group `id`, or every process when `id` is `None`.
    ///
    /// Each process names the group it was launched into, when that group was
    /// launched by this WineBridge or is the requested one.
    pub fn processes(&self, id: Option<&str>) -> Result<winebridge::ProcessList, Error> {
        let groups = match id {
            Some(id) => match Group::open(id)? {
                Some(group) => group
                    .pids()?
                    .into_iter()
                    .map(|pid| (pid, id.to_owned()))
                    .collect(),
                None => HashMap::new(),
            },
            None => self.launched()?,
        };
        Ok(winebridge::ProcessList {
            processes: ProcessSnapshot::new()?
                .filter(|process| id.is_none() || groups.contains_key(&process.pid()))
                .map(|process| process.to_proto(groups.get(&process.pid()).cloned()))
                .collect(),
        })
    }

    /// Maps each process in a group launched by this WineBridge to that group.
    fn launched(&self) -> Result<HashMap<u32, String>, Error> {
        let mut members = HashMap::new();
        for (id, group) in self.groups.lock().unwrap().iter() {
            for pid in group.pids()? {
                members.insert(pid, id.clone());
            }
        }
        Ok(members)
    }

    pub fn execute(&self, request: winebridge::LaunchProcessRequest) -> Result<u32, Error> {
        let mut groups = self.groups.lock().unwrap();
        if !groups.contains_key(&request.id) {
            let group = Group::create(&request.id)?;
            let association = JOBOBJECT_ASSOCIATE_COMPLETION_PORT {
                CompletionKey: std::ptr::null_mut(),
                CompletionPort: self.port.0,
            };
            unsafe {
                SetInformationJobObject(
                    group.0,
                    JobObjectAssociateCompletionPortInformation,
                    &association as *const _ as *const c_void,
                    size_of::<JOBOBJECT_ASSOCIATE_COMPLETION_PORT>() as u32,
                )?;
            }
            groups.insert(request.id.clone(), group);
        }
        let group = &groups[&request.id];
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

            if let Err(error) = AssignProcessToJobObject(group.0, process_info.0.hProcess) {
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

    /// Terminates every process in group `id`; a group that is not running is left alone.
    pub fn kill(&self, id: &str) -> Result<(), Error> {
        match Group::open(id)? {
            Some(group) => group.terminate(),
            None => Ok(()),
        }
    }

    /// Republishes the running groups after every process event.
    fn watch(&self) {
        loop {
            let (mut message, mut key) = (0, 0);
            let mut overlapped: *mut OVERLAPPED = std::ptr::null_mut();
            let event = unsafe {
                GetQueuedCompletionStatus(
                    self.port.0,
                    &mut message,
                    &mut key,
                    &mut overlapped,
                    INFINITE,
                )
            };
            if let Err(error) = event {
                tracing::error!(%error, "process events are no longer received");
                return;
            }
            match self.running_groups() {
                Ok(running) => {
                    self.running.send_if_modified(|current| {
                        let changed = *current != running;
                        *current = running;
                        changed
                    });
                }
                Err(error) => tracing::warn!(%error, "failed to list running groups"),
            }
        }
    }

    /// Lists the launched groups that have a running process, sorted by ID.
    fn running_groups(&self) -> Result<winebridge::RunningGroups, Error> {
        let mut ids = Vec::new();
        for (id, group) in self.groups.lock().unwrap().iter() {
            if !group.pids()?.is_empty() {
                ids.push(id.clone());
            }
        }
        ids.sort();
        Ok(winebridge::RunningGroups { ids })
    }
}
