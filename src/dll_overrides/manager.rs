use std::path::Path;
use windows_registry::Key;

use crate::registry::manager::{Data, Hive, KeyExtension, RegistryManager};

const DLL_OVERRIDES_SUBKEY: &str = "Software\\Wine\\DllOverrides";

#[derive(Debug, Clone, PartialEq)]
pub enum OverrideMode {
    NativeBuiltin,
    BuiltinNative,
    Native,
    Builtin,
    Disabled,
}

impl OverrideMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            OverrideMode::NativeBuiltin => "native,builtin",
            OverrideMode::BuiltinNative => "builtin,native",
            OverrideMode::Native => "native",
            OverrideMode::Builtin => "builtin",
            OverrideMode::Disabled => "disabled",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "native,builtin" => OverrideMode::NativeBuiltin,
            "builtin,native" => OverrideMode::BuiltinNative,
            "native" => OverrideMode::Native,
            "builtin" => OverrideMode::Builtin,
            "disabled" | "" => OverrideMode::Disabled,
            _ => OverrideMode::NativeBuiltin,
        }
    }

    pub fn to_proto_i32(&self) -> i32 {
        match self {
            OverrideMode::NativeBuiltin => 0,
            OverrideMode::BuiltinNative => 1,
            OverrideMode::Native => 2,
            OverrideMode::Builtin => 3,
            OverrideMode::Disabled => 4,
        }
    }

    pub fn from_proto_i32(v: i32) -> Self {
        match v {
            0 => OverrideMode::NativeBuiltin,
            1 => OverrideMode::BuiltinNative,
            2 => OverrideMode::Native,
            3 => OverrideMode::Builtin,
            4 => OverrideMode::Disabled,
            _ => OverrideMode::NativeBuiltin,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DllOverride {
    pub dll: String,
    pub mode: OverrideMode,
}

pub struct DllOverrideManager;

impl DllOverrideManager {
    fn subkey() -> &'static Path {
        Path::new(DLL_OVERRIDES_SUBKEY)
    }

    fn open_key() -> windows_registry::Result<Key> {
        RegistryManager.key(Hive::CurrentUser, Self::subkey())
    }

    fn ensure_key() -> windows_registry::Result<Key> {
        RegistryManager.create_key(Hive::CurrentUser, Self::subkey())
    }

    pub fn list(&self) -> windows_registry::Result<Vec<DllOverride>> {
        let key = Self::open_key()?;

        let overrides = key
            .values()?
            .filter_map(|(name, _)| {
                key.get_string(&name).ok().map(|s| DllOverride {
                    dll: name,
                    mode: OverrideMode::from_str(&s),
                })
            })
            .collect();

        Ok(overrides)
    }

    pub fn get(&self, dll: &str) -> windows_registry::Result<DllOverride> {
        let key = Self::open_key()?;
        let mode_str = key.get_string(dll)?;

        Ok(DllOverride {
            dll: dll.to_string(),
            mode: OverrideMode::from_str(&mode_str),
        })
    }

    pub fn set(&self, dll: &str, mode: OverrideMode) -> windows_registry::Result<()> {
        let key = Self::ensure_key()?;
        KeyExtension::create_value(&key, dll, Data::String(mode.as_str().to_string()))
    }

    pub fn delete(&self, dll: &str) -> windows_registry::Result<()> {
        let key = Self::open_key()?;
        key.remove_value(dll)
    }
}
