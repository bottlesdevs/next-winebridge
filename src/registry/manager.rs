use bottles_core::proto as proto_winebridge;
use proto_winebridge::registry_value::Value as ProtoValue;
use std::path::Path;
use windows_registry::*;

#[derive(Debug, Eq, PartialEq)]
pub enum Data {
    None(Vec<u8>),
    DWord(u32),
    QWord(u64),
    String(String),
    ExpandString(String),
    MultiString(Vec<String>),
    Bytes(Vec<u8>),
}

impl Hive {
    pub fn inner(&self) -> &Key {
        match self {
            Hive::ClassesRoot => CLASSES_ROOT,
            Hive::CurrentConfig => CURRENT_CONFIG,
            Hive::CurrentUser => CURRENT_USER,
            Hive::LocalMachine => LOCAL_MACHINE,
            Hive::Users => USERS,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Hive {
    ClassesRoot,
    CurrentConfig,
    CurrentUser,
    LocalMachine,
    Users,
}

impl TryFrom<proto_winebridge::RegistryHive> for Hive {
    type Error = &'static str;

    fn try_from(hive: proto_winebridge::RegistryHive) -> std::result::Result<Self, Self::Error> {
        match hive {
            proto_winebridge::RegistryHive::Unspecified => Err("registry hive is required"),
            proto_winebridge::RegistryHive::ClassesRoot => Ok(Self::ClassesRoot),
            proto_winebridge::RegistryHive::CurrentConfig => Ok(Self::CurrentConfig),
            proto_winebridge::RegistryHive::CurrentUser => Ok(Self::CurrentUser),
            proto_winebridge::RegistryHive::LocalMachine => Ok(Self::LocalMachine),
            proto_winebridge::RegistryHive::Users => Ok(Self::Users),
        }
    }
}

impl From<Hive> for proto_winebridge::RegistryHive {
    fn from(hive: Hive) -> Self {
        match hive {
            Hive::ClassesRoot => Self::ClassesRoot,
            Hive::CurrentConfig => Self::CurrentConfig,
            Hive::CurrentUser => Self::CurrentUser,
            Hive::LocalMachine => Self::LocalMachine,
            Hive::Users => Self::Users,
        }
    }
}

#[allow(dead_code)]
pub trait KeyExtension {
    fn get(hive: Hive, subkey: &Path) -> Result<Key> {
        hive.inner().open(subkey.display().to_string())
    }

    fn new(hive: Hive, subkey: &Path) -> Result<Key> {
        hive.inner().create(subkey.display().to_string())
    }

    fn delete(hive: Hive, subkey: &Path) -> Result<()> {
        hive.inner().remove_tree(subkey.display().to_string())
    }

    fn value(&self, name: &str) -> Result<Value>;
    fn values(&self) -> Result<Vec<(String, Value)>>;
    fn create_value(&self, name: &str, data: Data) -> Result<()>;
    fn rename_value(&self, old_name: &str, new_name: &str) -> Result<()>;

    fn as_registry_key(
        &self,
        hive: Hive,
        subkey: &Path,
    ) -> std::result::Result<proto_winebridge::RegistryKey, String>;
}

pub fn to_reg_data(value: ProtoValue) -> Data {
    match value {
        ProtoValue::None(value) => Data::None(value),
        ProtoValue::Binary(value) => Data::Bytes(value),
        ProtoValue::Dword(value) => Data::DWord(value),
        ProtoValue::Qword(value) => Data::QWord(value),
        ProtoValue::String(value) => Data::String(value),
        ProtoValue::ExpandString(value) => Data::ExpandString(value),
        ProtoValue::MultiString(value) => Data::MultiString(value.values),
    }
}

pub fn to_proto_reg_val(
    value: Value,
) -> std::result::Result<proto_winebridge::RegistryValue, String> {
    let value = match value.ty() {
        Type::Bytes => ProtoValue::Binary(value.to_vec()),
        Type::U32 => ProtoValue::Dword(u32::try_from(value).map_err(|error| error.to_string())?),
        Type::U64 => ProtoValue::Qword(u64::try_from(value).map_err(|error| error.to_string())?),
        Type::String => {
            ProtoValue::String(String::try_from(value).map_err(|error| error.to_string())?)
        }
        Type::ExpandString => {
            ProtoValue::ExpandString(String::try_from(value).map_err(|error| error.to_string())?)
        }
        Type::MultiString => ProtoValue::MultiString(proto_winebridge::RegistryMultiString {
            values: Vec::<String>::try_from(value).map_err(|error| error.to_string())?,
        }),
        Type::Other(0) => ProtoValue::None(value.to_vec()),
        Type::Other(_) => return Err("unsupported registry value type".to_string()),
    };

    Ok(proto_winebridge::RegistryValue { value: Some(value) })
}

impl KeyExtension for windows_registry::Key {
    fn as_registry_key(
        &self,
        hive: Hive,
        subkey: &Path,
    ) -> std::result::Result<proto_winebridge::RegistryKey, String> {
        let values: Vec<proto_winebridge::RegistryKeyValue> = self
            .values()
            .map_err(|error| error.to_string())?
            .map(|(name, value)| {
                Ok(proto_winebridge::RegistryKeyValue {
                    name,
                    value: Some(to_proto_reg_val(value)?),
                })
            })
            .collect::<std::result::Result<_, String>>()?;

        Ok(proto_winebridge::RegistryKey {
            hive: proto_winebridge::RegistryHive::from(hive) as i32,
            subkey: subkey.display().to_string(),
            values,
        })
    }

    fn value(&self, name: &str) -> Result<Value> {
        let value = self.get_value(name)?;
        Ok(value)
    }

    fn values(&self) -> Result<Vec<(String, Value)>> {
        let mut values = Vec::new();
        for value in self.values()? {
            values.push(value);
        }
        Ok(values)
    }

    fn create_value(&self, name: &str, data: Data) -> Result<()> {
        match data {
            Data::None(val) => self.set_bytes(name, Type::Other(0), &val),
            Data::Bytes(val) => self.set_bytes(name, Type::Bytes, &val),
            Data::DWord(val) => self.set_u32(name, val),
            Data::QWord(val) => self.set_u64(name, val),
            Data::String(val) => self.set_string(name, &val),
            Data::ExpandString(val) => self.set_expand_string(name, &val),
            Data::MultiString(val) => {
                let d: Vec<&str> = val.iter().map(|s| s.as_str()).collect();
                self.set_multi_string(name, &d)
            }
        }
    }

    fn rename_value(&self, old_name: &str, new_name: &str) -> Result<()> {
        let value = self.get_value(old_name)?;
        self.remove_value(old_name)?;
        self.set_value(new_name, &value)
    }
}

pub struct RegistryManager;

#[allow(dead_code)]
impl RegistryManager {
    pub fn value(&self, hive: Hive, subkey: &Path, name: &str) -> Result<Value> {
        let key = hive.inner().open(subkey.display().to_string())?;

        key.value(name)
    }

    pub fn key(&self, hive: Hive, subkey: &Path) -> Result<Key> {
        hive.inner().open(subkey.display().to_string())
    }

    pub fn create_key(&self, hive: Hive, subkey: &Path) -> Result<Key> {
        Key::new(hive, subkey)
    }

    pub fn delete_key(&self, hive: Hive, subkey: &Path) -> Result<()> {
        Key::delete(hive, subkey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn registry_value_round_trips_without_raw_type_data_pairs() {
        let proto = to_proto_reg_val(Value::from(42u32)).unwrap();
        let value = proto.value.unwrap();

        assert_eq!(value, ProtoValue::Dword(42));
        assert_eq!(to_reg_data(value), Data::DWord(42));
    }

    fn test_subkey() -> PathBuf {
        PathBuf::from("Software\\WineBridgeTest")
    }

    #[test]
    fn test_create_key() {
        let hive = Hive::CurrentUser;
        let subkey = test_subkey();
        assert!(Key::new(hive, &subkey).is_ok(), "Failed to create key");

        // Check if the key exists
        let key = hive.inner().open(&subkey.display().to_string());
        assert!(key.is_ok(), "Failed to open key");

        // Clean up
        hive.inner()
            .remove_tree(&subkey.display().to_string())
            .expect("Failed to delete test key");
    }

    #[test]
    fn test_get_key() {
        let hive = Hive::CurrentUser;
        let subkey = test_subkey();
        Key::new(hive, &subkey).expect("Failed to create key");

        // Get the key
        let key = Key::get(hive, &subkey);
        assert!(key.is_ok(), "Failed to open key");

        // Clean up
        hive.inner()
            .remove_tree(&subkey.display().to_string())
            .expect("Failed to delete test key");
    }

    #[test]
    fn test_delete_key() {
        let hive = Hive::CurrentUser;
        let subkey = test_subkey();
        Key::new(hive, &subkey).expect("Failed to create key");

        // Delete the key
        Key::delete(hive, &subkey).expect("Failed to delete key");

        // Check if the key is deleted
        let key = hive.inner().open(&subkey.display().to_string());
        assert!(key.is_err(), "Key still exists after deletion");
    }

    #[test]
    fn test_create_value() {
        let hive = Hive::CurrentUser;
        let subkey = test_subkey();

        let key = Key::new(hive, &subkey).expect("Failed to create key");

        // Set values
        key.create_value("TestDWord", Data::DWord(42))
            .expect("Failed to set DWord");
        key.create_value("TestString", Data::String("hello".to_string()))
            .expect("Failed to set String");

        // Get values
        let dword = key.get_u32("TestDWord").expect("Failed to get DWord");
        assert_eq!(dword, 42);

        let string = key.get_string("TestString").expect("Failed to get String");
        assert_eq!(string, "hello");

        key.remove_value("TestDWord")
            .expect("Failed to remove DWord");
        key.remove_value("TestString")
            .expect("Failed to remove String");
    }

    #[test]
    fn test_rename_value() {
        let hive = Hive::CurrentUser;
        let subkey = test_subkey();
        let key = Key::new(hive, &subkey).expect("Failed to open key");

        key.create_value("FromDWord", Data::DWord(42))
            .expect("Failed to set DWord");

        // Rename value
        key.rename_value("FromDWord", "ToDWord")
            .expect("Failed to rename value");
        let renamed = key.get_u32("ToDWord").expect("Failed to get renamed value");
        assert_eq!(renamed, 42);

        // Delete value
        key.remove_value("ToDWord").expect("Failed to delete value");
        assert!(key.value("ToDWord").is_err());
    }
}
