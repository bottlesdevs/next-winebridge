pub mod manager;

pub fn hive(hive: i32) -> Result<bottles_core::RegistryHive, tonic::Status> {
    match bottles_core::RegistryHive::try_from(hive) {
        Ok(bottles_core::RegistryHive::Unspecified) => {
            Err(tonic::Status::invalid_argument("registry hive is required"))
        }
        Ok(hive) => Ok(hive),
        Err(_) => Err(tonic::Status::invalid_argument("invalid registry hive")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hive_rejects_unspecified() {
        assert!(super::hive(0).is_err());
        assert_eq!(
            super::hive(3).unwrap(),
            bottles_core::RegistryHive::CurrentUser
        );
    }
}
