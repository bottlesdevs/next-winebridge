use windows_registry::*;

pub enum Hive {
    ClassesRoot,
    CurrentConfig,
    CurrentUser,
    LocalMachine,
    Users,
}

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
    pub fn key(&self) -> &Key {
        match self {
            Hive::ClassesRoot => CLASSES_ROOT,
            Hive::CurrentConfig => CURRENT_CONFIG,
            Hive::CurrentUser => CURRENT_USER,
            Hive::LocalMachine => LOCAL_MACHINE,
            Hive::Users => USERS,
        }
    }
}

pub struct RegistryManager;

impl RegistryManager {
    pub fn value(&self, hive: &Hive, subkey: &str, name: &str) -> Result<Value> {
        let key = hive.key().open(subkey)?;

        key.get_value(name)
    }

    pub fn set_value(&self, hive: &Hive, subkey: &str, name: &str, data: Data) -> Result<()> {
        let key = hive.key().create(subkey)?;

        match data {
            Data::Bytes(val) => key.set_bytes(name, Type::Bytes, &val),
            Data::DWord(val) => key.set_u32(name, val),
            Data::QWord(val) => key.set_u64(name, val),
            Data::String(val) => key.set_string(name, &val),
            Data::ExpandString(val) => key.set_expand_string(name, &val),
            Data::MultiString(val) => {
                let d: Vec<&str> = val.iter().map(|s| s.as_str()).collect();
                key.set_multi_string(name, &d)
            }
            _ => unimplemented!(),
        }
    }

    pub fn delete_value(&self, hive: &Hive, subkey: &str, name: &str) -> Result<()> {
        let key = hive.key().open(subkey)?;

        key.remove_value(name)
    }

    pub fn key(&self, hive: &Hive, subkey: &str) -> Result<Key> {
        hive.key().open(subkey)
    }

    pub fn create_key(&self, hive: &Hive, subkey: &str) -> Result<Key> {
        hive.key().create(subkey)
    }

    pub fn delete_key(&self, hive: &Hive, subkey: &str) -> Result<()> {
        hive.key().remove_tree(subkey)
    }
}
