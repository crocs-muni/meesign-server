use super::{
    models::{Device, Group},
    persistance_error::PersistenceError,
};

#[tonic::async_trait]
pub trait MeesignRepo {
    /* Devices */
    async fn add_device(
        &self,
        identifier: &[u8],
        name: &str,
        certificate: &[u8],
    ) -> Result<(), PersistenceError>;
    async fn get_devices(&self) -> Result<Vec<Device>, PersistenceError>;
    async fn activate_device(&self, identifier: &Vec<u8>) -> Result<(), PersistenceError>;

    // async fn get_device(&self, identifier: &Vec<u8>) -> Option<Device>;

    // /* Groups */
    // async fn add_group<'a>(
    //     &self,
    //     name: &str,
    //     devices: &[Vec<u8>],
    //     threshold: u32,
    //     protocol: ProtocolType,
    // ) -> Result<Group, ()>;
    // async fn get_group(&self, group_identifier: &Vec<u8>) -> Option<Group>;
    async fn get_groups(&self) -> Result<Vec<Group>, PersistenceError>;
}
