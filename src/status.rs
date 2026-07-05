use tonic::Status;
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_INVALID_DATA,
    ERROR_INVALID_PARAMETER, ERROR_PATH_NOT_FOUND, ERROR_SERVICE_ALREADY_RUNNING,
    ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_EXISTS, ERROR_SERVICE_NOT_ACTIVE,
};
use windows::core::{Error, HRESULT};

pub fn windows(error: Error) -> Status {
    let code = error.code();
    if code == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0)
        || code == HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0)
        || code == HRESULT::from_win32(ERROR_SERVICE_DOES_NOT_EXIST.0)
    {
        Status::not_found(error.to_string())
    } else if code == HRESULT::from_win32(ERROR_ALREADY_EXISTS.0)
        || code == HRESULT::from_win32(ERROR_SERVICE_EXISTS.0)
    {
        Status::already_exists(error.to_string())
    } else if code == HRESULT::from_win32(ERROR_ACCESS_DENIED.0) {
        Status::permission_denied(error.to_string())
    } else if code == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) {
        Status::invalid_argument(error.to_string())
    } else if code == HRESULT::from_win32(ERROR_SERVICE_ALREADY_RUNNING.0)
        || code == HRESULT::from_win32(ERROR_SERVICE_NOT_ACTIVE.0)
    {
        Status::failed_precondition(error.to_string())
    } else if code == HRESULT::from_win32(ERROR_INVALID_DATA.0) {
        Status::data_loss(error.to_string())
    } else {
        Status::internal(error.to_string())
    }
}

pub fn io(error: std::io::Error) -> Status {
    use std::io::ErrorKind;

    match error.kind() {
        ErrorKind::NotFound => Status::not_found(error.to_string()),
        ErrorKind::AlreadyExists => Status::already_exists(error.to_string()),
        ErrorKind::PermissionDenied => Status::permission_denied(error.to_string()),
        ErrorKind::InvalidInput => Status::invalid_argument(error.to_string()),
        ErrorKind::InvalidData => Status::data_loss(error.to_string()),
        ErrorKind::DirectoryNotEmpty | ErrorKind::IsADirectory | ErrorKind::NotADirectory => {
            Status::failed_precondition(error.to_string())
        }
        _ => Status::internal(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn maps_common_windows_errors() {
        let error = |code| Error::from_hresult(HRESULT::from_win32(code));
        assert_eq!(
            windows(error(ERROR_FILE_NOT_FOUND.0)).code(),
            Code::NotFound
        );
        assert_eq!(
            windows(error(ERROR_ACCESS_DENIED.0)).code(),
            Code::PermissionDenied
        );
        assert_eq!(
            windows(error(ERROR_ALREADY_EXISTS.0)).code(),
            Code::AlreadyExists
        );
    }

    #[test]
    fn maps_common_io_errors() {
        let error = |kind| std::io::Error::from(kind);
        assert_eq!(
            io(error(std::io::ErrorKind::NotFound)).code(),
            Code::NotFound
        );
        assert_eq!(
            io(error(std::io::ErrorKind::PermissionDenied)).code(),
            Code::PermissionDenied
        );
        assert_eq!(
            io(error(std::io::ErrorKind::AlreadyExists)).code(),
            Code::AlreadyExists
        );
    }
}
