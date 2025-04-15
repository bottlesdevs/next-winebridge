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
