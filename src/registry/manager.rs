use bottles_core::proto;
use std::{ops::Deref, path::Path, str::FromStr};
use windows_registry::*;

#[derive(Debug, Eq, PartialEq)]
pub enum Data {
    DWord(u32),
    QWord(u64),
    String(String),
    ExpandString(String),
    MultiString(Vec<String>),
    Bytes(Vec<u8>),
    Other,
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

    pub fn to_string(&self) -> String {
        match self {
            Hive::ClassesRoot => "HKCR".to_string(),
            Hive::CurrentConfig => "HKCC".to_string(),
            Hive::CurrentUser => "HKCU".to_string(),
            Hive::LocalMachine => "HKLM".to_string(),
            Hive::Users => "HKU".to_string(),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Hive {
    ClassesRoot,
    CurrentConfig,
    CurrentUser,
    LocalMachine,
    Users,
}

impl FromStr for Hive {
    type Err = &'static str;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_uppercase().as_str() {
            "CLASSESROOT" | "HKCR" => Ok(Hive::ClassesRoot),
            "CURRENTCONFIG" | "HKCC" => Ok(Hive::CurrentConfig),
            "CURRENTUSER" | "HKCU" => Ok(Hive::CurrentUser),
            "LOCALMACHINE" | "HKLM" => Ok(Hive::LocalMachine),
            "USERS" | "HKU" => Ok(Hive::Users),
            _ => Err("invalid registry hive"),
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

    fn as_registry_key(&self, hive: Hive, subkey: &Path) -> proto::RegistryKey;
}

pub fn to_reg_data(ty: proto::RegistryValueType, data: Vec<u8>) -> Data {
    match ty {
        proto::RegistryValueType::RegBinary => Data::Bytes(data),
        proto::RegistryValueType::RegDword => {
            let val = u32::from_le_bytes(data.try_into().unwrap());
            Data::DWord(val)
        }
        proto::RegistryValueType::RegQword => {
            let val = u64::from_le_bytes(data.try_into().unwrap());
            Data::QWord(val)
        }
        proto::RegistryValueType::RegSz => {
            let val = String::from_utf8(data).unwrap();
            Data::String(val)
        }
        proto::RegistryValueType::RegExpandSz => {
            let val = String::from_utf8(data).unwrap();
            Data::ExpandString(val)
        }
        proto::RegistryValueType::RegMultiSz => {
            let val = String::from_utf8(data).unwrap();
            let strings: Vec<String> = val.split('\0').map(|s| s.to_string()).collect();
            Data::MultiString(strings)
        }
        proto::RegistryValueType::RegNone => Data::Other,
    }
}

pub fn to_proto_reg_val(value: Value) -> proto::RegistryValue {
    let ty = match value.ty() {
        Type::Bytes => proto::RegistryValueType::RegBinary,
        Type::U32 => proto::RegistryValueType::RegDword,
        Type::U64 => proto::RegistryValueType::RegQword,
        Type::String => proto::RegistryValueType::RegSz,
        Type::ExpandString => proto::RegistryValueType::RegExpandSz,
        Type::MultiString => proto::RegistryValueType::RegMultiSz,
        Type::Other(_) => proto::RegistryValueType::RegNone,
    };

    let val = value.deref();
    proto::RegistryValue {
        r#type: ty as i32,
        data: val.to_vec(),
    }
}

impl KeyExtension for windows_registry::Key {
    fn as_registry_key(&self, hive: Hive, subkey: &Path) -> proto::RegistryKey {
        let values: Vec<proto::RegistryKeyValue> = self
            .values()
            .unwrap()
            .map(|(name, value)| proto::RegistryKeyValue {
                name,
                value: Some(to_proto_reg_val(value)),
            })
            .collect();

        proto::RegistryKey {
            hive: hive.to_string(),
            subkey: subkey.display().to_string(),
            values,
        }
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
            Data::Bytes(val) => self.set_bytes(name, Type::Bytes, &val),
            Data::DWord(val) => self.set_u32(name, val),
            Data::QWord(val) => self.set_u64(name, val),
            Data::String(val) => self.set_string(name, &val),
            Data::ExpandString(val) => self.set_expand_string(name, &val),
            Data::MultiString(val) => {
                let d: Vec<&str> = val.iter().map(|s| s.as_str()).collect();
                self.set_multi_string(name, &d)
            }
            _ => unreachable!(),
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
