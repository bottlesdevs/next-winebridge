pub mod manager;

use bottles_core::proto as winebridge;
use tonic::Status;
use windows::Win32::System::Services::{
    SERVICE_AUTO_START, SERVICE_BOOT_START, SERVICE_DEMAND_START, SERVICE_DISABLED,
    SERVICE_SYSTEM_START,
};

pub fn to_proto(service: manager::ServiceInfo) -> Result<winebridge::Service, Status> {
    let state = winebridge::ServiceState::try_from(service.state as i32)
        .map_err(|_| Status::data_loss(format!("unknown service state {}", service.state)))?;
    if state == winebridge::ServiceState::ServiceUnspecified {
        return Err(Status::data_loss("service state is unspecified"));
    }

    let start_type = match service.start_type {
        value if value == SERVICE_BOOT_START.0 => winebridge::ServiceStartType::ServiceBootStart,
        value if value == SERVICE_SYSTEM_START.0 => {
            winebridge::ServiceStartType::ServiceSystemStart
        }
        value if value == SERVICE_AUTO_START.0 => winebridge::ServiceStartType::ServiceAutoStart,
        value if value == SERVICE_DEMAND_START.0 => {
            winebridge::ServiceStartType::ServiceDemandStart
        }
        value if value == SERVICE_DISABLED.0 => winebridge::ServiceStartType::ServiceDisabled,
        value => {
            return Err(Status::data_loss(format!(
                "unknown service start type {value}"
            )));
        }
    };

    Ok(winebridge::Service {
        name: service.name,
        display_name: service.display_name,
        state: state as i32,
        start_type: start_type as i32,
    })
}

pub fn start_type(value: i32) -> Result<u32, Status> {
    match winebridge::ServiceStartType::try_from(value)
        .map_err(|_| Status::invalid_argument("invalid service start type"))?
    {
        winebridge::ServiceStartType::ServiceStartUnspecified => {
            Err(Status::invalid_argument("service start type is required"))
        }
        winebridge::ServiceStartType::ServiceBootStart => Ok(SERVICE_BOOT_START.0),
        winebridge::ServiceStartType::ServiceSystemStart => Ok(SERVICE_SYSTEM_START.0),
        winebridge::ServiceStartType::ServiceAutoStart => Ok(SERVICE_AUTO_START.0),
        winebridge::ServiceStartType::ServiceDemandStart => Ok(SERVICE_DEMAND_START.0),
        winebridge::ServiceStartType::ServiceDisabled => Ok(SERVICE_DISABLED.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_windows_service_values() {
        let service = to_proto(manager::ServiceInfo {
            name: "svc".into(),
            display_name: "Service".into(),
            state: 4,
            start_type: SERVICE_AUTO_START.0,
        })
        .unwrap();

        assert_eq!(service.state(), winebridge::ServiceState::ServiceRunning);
        assert_eq!(
            service.start_type(),
            winebridge::ServiceStartType::ServiceAutoStart
        );
        assert!(start_type(0).is_err());
    }
}
