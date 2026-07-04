pub mod manager;

impl TryFrom<i32> for manager::Hive {
    type Error = tonic::Status;

    fn try_from(hive: i32) -> Result<Self, Self::Error> {
        bottles_core::RegistryHive::try_from(hive)
            .map_err(|_| tonic::Status::invalid_argument("invalid registry hive"))?
            .try_into()
            .map_err(tonic::Status::invalid_argument)
    }
}
