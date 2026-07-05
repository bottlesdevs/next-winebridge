use bottles_core::proto::{self as winebridge, RegistryHive, registry_value::Value as ProtoValue};
use tonic::Status;
use windows_registry::{
    CLASSES_ROOT, CURRENT_CONFIG, CURRENT_USER, Key, LOCAL_MACHINE, Type, USERS, Value,
};

use crate::status;

pub fn create_key(hive: i32, subkey: &str) -> Result<(), Status> {
    let root = resolve_root(hive, subkey)?;
    root.create(subkey).map(drop).map_err(status::windows)
}

pub fn delete_tree(hive: i32, subkey: &str) -> Result<(), Status> {
    let root = resolve_root(hive, subkey)?;
    root.remove_tree(subkey).map_err(status::windows)
}

pub fn get_key(hive: i32, subkey: &str) -> Result<winebridge::RegistryKey, Status> {
    let root = resolve_root(hive, subkey)?;
    let key = root.open(subkey).map_err(status::windows)?;
    let values = key
        .values()
        .map_err(status::windows)?
        .map(|(name, value)| {
            Ok(winebridge::RegistryKeyValue {
                name,
                value: Some(to_proto(value)?),
            })
        })
        .collect::<Result<_, Status>>()?;

    Ok(winebridge::RegistryKey {
        hive,
        subkey: subkey.to_string(),
        values,
    })
}

pub fn get_value(hive: i32, subkey: &str, name: &str) -> Result<winebridge::RegistryValue, Status> {
    validate_name(name)?;
    let root = resolve_root(hive, subkey)?;
    to_proto(
        root.open(subkey)
            .map_err(status::windows)?
            .get_value(name)
            .map_err(status::windows)?,
    )
}

pub fn set_value(hive: i32, subkey: &str, name: &str, value: ProtoValue) -> Result<(), Status> {
    validate_name(name)?;
    let root = resolve_root(hive, subkey)?;
    let key = root.open(subkey).map_err(status::windows)?;

    match value {
        ProtoValue::None(value) => key.set_bytes(name, Type::Other(0), &value),
        ProtoValue::Binary(value) => key.set_bytes(name, Type::Bytes, &value),
        ProtoValue::Dword(value) => key.set_u32(name, value),
        ProtoValue::Qword(value) => key.set_u64(name, value),
        ProtoValue::String(value) => {
            validate_string(&value)?;
            key.set_string(name, value)
        }
        ProtoValue::ExpandString(value) => {
            validate_string(&value)?;
            key.set_expand_string(name, value)
        }
        ProtoValue::MultiString(value) => {
            if value
                .values
                .iter()
                .any(|value| value.is_empty() || value.contains('\0'))
            {
                return Err(Status::invalid_argument(
                    "registry multi-string values must be non-empty and contain no NUL bytes",
                ));
            }
            let values: Vec<_> = value.values.iter().map(String::as_str).collect();
            key.set_multi_string(name, &values)
        }
    }
    .map_err(status::windows)
}

pub fn delete_value(hive: i32, subkey: &str, name: &str) -> Result<(), Status> {
    validate_name(name)?;
    let root = resolve_root(hive, subkey)?;
    root.open(subkey)
        .map_err(status::windows)?
        .remove_value(name)
        .map_err(status::windows)
}

fn resolve_root(hive: i32, subkey: &str) -> Result<&'static Key, Status> {
    if subkey.is_empty() || subkey.contains('\0') {
        return Err(Status::invalid_argument(
            "registry subkey must be non-empty and contain no NUL bytes",
        ));
    }

    let hive = RegistryHive::try_from(hive)
        .map_err(|_| Status::invalid_argument("invalid registry hive"))?;
    Ok(match hive {
        RegistryHive::ClassesRoot => CLASSES_ROOT,
        RegistryHive::CurrentConfig => CURRENT_CONFIG,
        RegistryHive::CurrentUser => CURRENT_USER,
        RegistryHive::LocalMachine => LOCAL_MACHINE,
        RegistryHive::Users => USERS,
        RegistryHive::Unspecified => {
            return Err(Status::invalid_argument("registry hive is required"));
        }
    })
}

fn validate_name(name: &str) -> Result<(), Status> {
    if name.contains('\0') {
        Err(Status::invalid_argument(
            "registry value name must contain no NUL bytes",
        ))
    } else {
        Ok(())
    }
}

fn validate_string(value: &str) -> Result<(), Status> {
    if value.contains('\0') {
        Err(Status::invalid_argument(
            "registry string value must contain no NUL bytes",
        ))
    } else {
        Ok(())
    }
}

fn to_proto(value: Value) -> Result<winebridge::RegistryValue, Status> {
    let value = match value.ty() {
        Type::Bytes => ProtoValue::Binary(value.to_vec()),
        Type::U32 => ProtoValue::Dword(u32::try_from(value).map_err(status::windows)?),
        Type::U64 => ProtoValue::Qword(u64::try_from(value).map_err(status::windows)?),
        Type::String => ProtoValue::String(String::try_from(value).map_err(status::windows)?),
        Type::ExpandString => {
            ProtoValue::ExpandString(String::try_from(value).map_err(status::windows)?)
        }
        Type::MultiString => {
            if !value.len().is_multiple_of(2) {
                return Err(Status::data_loss(
                    "registry multi-string has an odd byte length",
                ));
            }

            let mut wide = value.as_wide();
            while wide.last() == Some(&0) {
                wide = &wide[..wide.len() - 1];
            }
            let values = if wide.is_empty() {
                Vec::new()
            } else {
                wide.split(|character| *character == 0)
                    .map(|value| {
                        String::from_utf16(value)
                            .map_err(|error| Status::data_loss(error.to_string()))
                    })
                    .collect::<Result<_, _>>()?
            };

            ProtoValue::MultiString(winebridge::RegistryMultiString { values })
        }
        Type::Other(0) => ProtoValue::None(value.to_vec()),
        Type::Other(kind) => {
            return Err(Status::unimplemented(format!(
                "unsupported registry value type {kind}"
            )));
        }
    };

    Ok(winebridge::RegistryValue { value: Some(value) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    const TEST_SUBKEY: &str = "Software\\WineBridgeTest";

    #[test]
    fn rejects_invalid_registry_addresses() {
        assert_eq!(
            resolve_root(0, TEST_SUBKEY).unwrap_err().code(),
            Code::InvalidArgument
        );
        assert_eq!(
            resolve_root(RegistryHive::CurrentUser as i32, "")
                .unwrap_err()
                .code(),
            Code::InvalidArgument
        );
        assert_eq!(
            validate_name("bad\0name").unwrap_err().code(),
            Code::InvalidArgument
        );
    }

    #[test]
    fn decodes_multi_strings_without_terminator_entries() {
        let bytes: Vec<_> = "one\0two\0\0"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        let mut value = Value::from(bytes.as_slice());
        value.set_ty(Type::MultiString);

        assert_eq!(
            to_proto(value).unwrap().value,
            Some(ProtoValue::MultiString(winebridge::RegistryMultiString {
                values: vec!["one".into(), "two".into()],
            }))
        );

        let mut malformed = Value::from([0]);
        malformed.set_ty(Type::MultiString);
        assert_eq!(to_proto(malformed).unwrap_err().code(), Code::DataLoss);

        let mut unsupported = Value::from([0]);
        unsupported.set_ty(Type::Other(42));
        assert_eq!(
            to_proto(unsupported).unwrap_err().code(),
            Code::Unimplemented
        );
    }

    #[test]
    fn registry_crud_round_trips_supported_values() {
        let _ = CURRENT_USER.remove_tree(TEST_SUBKEY);
        create_key(RegistryHive::CurrentUser as i32, TEST_SUBKEY).unwrap();

        let values = [
            ("", ProtoValue::None(vec![1, 2])),
            ("binary", ProtoValue::Binary(vec![3, 4])),
            ("dword", ProtoValue::Dword(42)),
            ("qword", ProtoValue::Qword(u64::MAX)),
            ("string", ProtoValue::String("hello".into())),
            ("expand", ProtoValue::ExpandString("%PATH%".into())),
            (
                "multi",
                ProtoValue::MultiString(winebridge::RegistryMultiString {
                    values: vec!["one".into(), "two".into()],
                }),
            ),
        ];

        for (name, value) in &values {
            set_value(
                RegistryHive::CurrentUser as i32,
                TEST_SUBKEY,
                name,
                value.clone(),
            )
            .unwrap();
            assert_eq!(
                get_value(RegistryHive::CurrentUser as i32, TEST_SUBKEY, name)
                    .unwrap()
                    .value,
                Some(value.clone())
            );
        }

        assert_eq!(
            get_key(RegistryHive::CurrentUser as i32, TEST_SUBKEY)
                .unwrap()
                .values
                .len(),
            values.len()
        );
        delete_value(RegistryHive::CurrentUser as i32, TEST_SUBKEY, "").unwrap();
        create_key(
            RegistryHive::CurrentUser as i32,
            &format!("{TEST_SUBKEY}\\Child"),
        )
        .unwrap();
        delete_tree(RegistryHive::CurrentUser as i32, TEST_SUBKEY).unwrap();
        assert!(CURRENT_USER.open(TEST_SUBKEY).is_err());
    }
}
