use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use windows::Win32::System::Services::*;
const DELETE: u32 = 0x00010000;
use windows::core::{Error, PCWSTR, PWSTR};

fn to_wide(s: &str) -> Vec<u16> {
    OsString::from(s).encode_wide().chain(Some(0)).collect()
}

fn from_wide(ptr: PWSTR) -> String {
    if ptr.is_null() {
        return String::new();
    }
    unsafe {
        let len = (0..).take_while(|&i| *ptr.0.add(i) != 0).count();
        OsString::from_wide(std::slice::from_raw_parts(ptr.0, len))
            .to_string_lossy()
            .into_owned()
    }
}

pub struct ScmHandle(SC_HANDLE);

impl Drop for ScmHandle {
    fn drop(&mut self) {
        unsafe { CloseServiceHandle(self.0).ok() };
    }
}

pub struct ServiceHandle(SC_HANDLE);

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        unsafe { CloseServiceHandle(self.0).ok() };
    }
}

#[derive(Debug, Clone)]
pub struct ServiceInfo {
    pub name: String,
    pub display_name: String,
    pub state: u32,
    pub start_type: u32,
}

pub struct ServiceManager;

impl ServiceManager {
    fn open_scm(access: u32) -> Result<ScmHandle, Error> {
        let handle = unsafe {
            OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access)
        }?;
        Ok(ScmHandle(handle))
    }

    fn open_service(scm: &ScmHandle, name: &str, access: u32) -> Result<ServiceHandle, Error> {
        let wide = to_wide(name);
        let handle = unsafe {
            OpenServiceW(scm.0, PCWSTR(wide.as_ptr()), access)
        }?;
        Ok(ServiceHandle(handle))
    }

    fn query_start_type(&self, scm: &ScmHandle, name: &str) -> Result<u32, Error> {
        let svc = Self::open_service(scm, name, SERVICE_QUERY_CONFIG)?;

        let mut bytes_needed: u32 = 0;
        unsafe {
            let _ = QueryServiceConfigW(svc.0, None, 0, &mut bytes_needed);
        }

        let mut buf: Vec<u8> = vec![0u8; bytes_needed as usize];
        unsafe {
            QueryServiceConfigW(
                svc.0,
                Some(buf.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW),
                bytes_needed,
                &mut bytes_needed,
            )?;
        }

        let config = unsafe { &*(buf.as_ptr() as *const QUERY_SERVICE_CONFIGW) };
        Ok(config.dwStartType.0)
    }

    pub fn list_services(&self) -> Result<Vec<ServiceInfo>, Error> {
        let scm = Self::open_scm(SC_MANAGER_ENUMERATE_SERVICE)?;

        let mut bytes_needed: u32 = 0;
        let mut services_returned: u32 = 0;
        let mut resume_handle: u32 = 0;

        unsafe {
            let _ = EnumServicesStatusExW(
                scm.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                None,
                &mut bytes_needed,
                &mut services_returned,
                Some(&mut resume_handle),
                PCWSTR::null(),
            );
        }

        let mut buf: Vec<u8> = vec![0u8; bytes_needed as usize];
        resume_handle = 0;

        unsafe {
            EnumServicesStatusExW(
                scm.0,
                SC_ENUM_PROCESS_INFO,
                SERVICE_WIN32,
                SERVICE_STATE_ALL,
                Some(&mut buf),
                &mut bytes_needed,
                &mut services_returned,
                Some(&mut resume_handle),
                PCWSTR::null(),
            )?;
        }

        let mut result = Vec::with_capacity(services_returned as usize);
        let ptr = buf.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW;

        for i in 0..services_returned as usize {
            let entry = unsafe { &*ptr.add(i) };
            let name = from_wide(PWSTR(entry.lpServiceName.0 as *mut u16));
            let display_name = from_wide(PWSTR(entry.lpDisplayName.0 as *mut u16));
            let state = entry.ServiceStatusProcess.dwCurrentState.0;
            let start_type = self.query_start_type(&scm, &name).unwrap_or(0);

            result.push(ServiceInfo { name, display_name, state, start_type });
        }

        Ok(result)
    }

    pub fn get_status(&self, name: &str) -> Result<u32, Error> {
        let scm = Self::open_scm(SC_MANAGER_CONNECT)?;
        let svc = Self::open_service(&scm, name, SERVICE_QUERY_STATUS)?;

        let mut status = SERVICE_STATUS_PROCESS::default();
        let mut bytes_needed: u32 = 0;

        unsafe {
            QueryServiceStatusEx(
                svc.0,
                SC_STATUS_PROCESS_INFO,
                Some(std::slice::from_raw_parts_mut(
                    &mut status as *mut _ as *mut u8,
                    std::mem::size_of::<SERVICE_STATUS_PROCESS>(),
                )),
                &mut bytes_needed,
            )?;
        }

        Ok(status.dwCurrentState.0)
    }

    pub fn start(&self, name: &str) -> Result<(), Error> {
        let scm = Self::open_scm(SC_MANAGER_CONNECT)?;
        let svc = Self::open_service(&scm, name, SERVICE_START)?;
        unsafe { StartServiceW(svc.0, None) }
    }

    pub fn stop(&self, name: &str) -> Result<(), Error> {
        let scm = Self::open_scm(SC_MANAGER_CONNECT)?;
        let svc = Self::open_service(&scm, name, SERVICE_STOP)?;
        let mut status = SERVICE_STATUS::default();
        unsafe { ControlService(svc.0, SERVICE_CONTROL_STOP, &mut status) }
    }

    pub fn create(
        &self,
        name: &str,
        display_name: &str,
        binary_path: &str,
        start_type: u32,
    ) -> Result<(), Error> {
        let scm = Self::open_scm(SC_MANAGER_CREATE_SERVICE)?;

        let name_w = to_wide(name);
        let display_w = to_wide(display_name);
        let path_w = to_wide(binary_path);

        let handle = unsafe {
            CreateServiceW(
                scm.0,
                PCWSTR(name_w.as_ptr()),
                PCWSTR(display_w.as_ptr()),
                SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_START_TYPE(start_type),
                SERVICE_ERROR_NORMAL,
                PCWSTR(path_w.as_ptr()),
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
            )
        }?;

        unsafe { CloseServiceHandle(handle).ok() };
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<(), Error> {
        let scm = Self::open_scm(SC_MANAGER_CONNECT)?;
        let svc = Self::open_service(&scm, name, DELETE)?;
        unsafe { DeleteService(svc.0) }
    }
}
