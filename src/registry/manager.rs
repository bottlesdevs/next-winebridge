use std::path::Path;
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

#[derive(Debug, Copy, Clone)]
pub enum Hive {
    ClassesRoot,
    CurrentConfig,
    CurrentUser,
    LocalMachine,
    Users,
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

trait KeyExtension {
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
}

impl KeyExtension for windows_registry::Key {
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

impl RegistryManager {
    pub fn value(&self, hive: &Hive, subkey: &str, name: &str) -> Result<Value> {
        let key = hive.inner().open(subkey)?;

        key.value(name)
    }

    pub fn key(&self, hive: &Hive, subkey: &str) -> Result<Key> {
        hive.inner().open(subkey)
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
