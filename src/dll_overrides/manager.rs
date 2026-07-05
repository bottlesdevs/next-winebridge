use bottles_core::proto::{DllOverride, DllOverrideMode};
use windows::Win32::Foundation::ERROR_INVALID_DATA;
use windows::core::{Error, HRESULT};
use windows_registry::{CURRENT_USER, Key};

const DLL_OVERRIDES_SUBKEY: &str = "Software\\Wine\\DllOverrides";

fn invalid_data() -> Error {
    Error::from_hresult(HRESULT::from_win32(ERROR_INVALID_DATA.0))
}

fn parse_mode(value: &str) -> windows_registry::Result<DllOverrideMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "native,builtin" => Ok(DllOverrideMode::NativeBuiltin),
        "builtin,native" => Ok(DllOverrideMode::BuiltinNative),
        "native" => Ok(DllOverrideMode::Native),
        "builtin" => Ok(DllOverrideMode::Builtin),
        "disabled" | "" => Ok(DllOverrideMode::Disabled),
        _ => Err(invalid_data()),
    }
}

fn mode_value(mode: DllOverrideMode) -> windows_registry::Result<&'static str> {
    match mode {
        DllOverrideMode::NativeBuiltin => Ok("native,builtin"),
        DllOverrideMode::BuiltinNative => Ok("builtin,native"),
        DllOverrideMode::Native => Ok("native"),
        DllOverrideMode::Builtin => Ok("builtin"),
        DllOverrideMode::Disabled => Ok("disabled"),
        DllOverrideMode::Unspecified => Err(invalid_data()),
    }
}

pub struct DllOverrideManager;

impl DllOverrideManager {
    fn open_key() -> windows_registry::Result<Key> {
        CURRENT_USER.open(DLL_OVERRIDES_SUBKEY)
    }

    fn ensure_key() -> windows_registry::Result<Key> {
        CURRENT_USER.create(DLL_OVERRIDES_SUBKEY)
    }

    pub fn list(&self) -> windows_registry::Result<Vec<DllOverride>> {
        Self::open_key()?
            .values()?
            .map(|(dll, value)| {
                Ok(DllOverride {
                    dll,
                    mode: parse_mode(&String::try_from(value)?)? as i32,
                })
            })
            .collect()
    }

    pub fn get(&self, dll: &str) -> windows_registry::Result<DllOverride> {
        let mode = parse_mode(&Self::open_key()?.get_string(dll)?)?;
        Ok(DllOverride {
            dll: dll.to_string(),
            mode: mode as i32,
        })
    }

    pub fn set(&self, dll: &str, mode: DllOverrideMode) -> windows_registry::Result<()> {
        Self::ensure_key()?.set_string(dll, mode_value(mode)?)
    }

    pub fn delete(&self, dll: &str) -> windows_registry::Result<()> {
        Self::open_key()?.remove_value(dll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_known_override_modes() {
        assert_eq!(parse_mode("native").unwrap(), DllOverrideMode::Native);
        assert!(parse_mode("unexpected").is_err());
        assert!(mode_value(DllOverrideMode::Unspecified).is_err());
    }
}
