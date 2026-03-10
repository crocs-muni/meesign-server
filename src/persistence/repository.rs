use super::{
    enums::{DeviceKind, KeyType, ProtocolType, TaskType},
    error::PersistenceError,
    models::{Device, Group, Participant, Task},
};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[cfg(test)]
use mockall::automock;

#[cfg_attr(test, automock)]
#[async_trait]
pub trait Repository {
    /* Devices */
    async fn add_device(
        &self,
        identifier: &[u8],
        name: &str,
        kind: &DeviceKind,
        certificate: &[u8],
    ) -> Result<Device, PersistenceError>;

    async fn get_devices(&self) -> Result<Vec<Device>, PersistenceError>;

    async fn get_group_participants(
        &self,
        group_id: &[u8],
    ) -> Result<Vec<Participant>, PersistenceError>;

    async fn get_tasks_participants(
        &self,
        task_ids: &[Uuid],
    ) -> Result<Vec<(Uuid, Participant)>, PersistenceError>;

    /* Groups */
    async fn add_group<'a>(
        &self,
        identifier: &[u8],
        group_task_id: &Uuid,
        name: &str,
        threshold: u32,
        protocol: ProtocolType,
        key_type: KeyType,
        certificate: Option<&'a [u8]>,
        note: Option<&'a str>,
    ) -> Result<Group, PersistenceError>;

    async fn get_group(&self, group_identifier: &[u8]) -> Result<Option<Group>, PersistenceError>;

    async fn get_groups(&self) -> Result<Vec<Group>, PersistenceError>;

    async fn get_device_groups(&self, identifier: &[u8]) -> Result<Vec<Group>, PersistenceError>;

    /* Tasks */
    async fn create_group_task<'a>(
        &self,
        id: Option<&'a Uuid>,
        participants: &[(&'a [u8], u32)],
        threshold: u32,
        name: &str,
        protocol_type: ProtocolType,
        key_type: KeyType,
        request: &[u8],
        note: Option<&'a str>,
    ) -> Result<Task, PersistenceError>;

    async fn create_threshold_task<'a>(
        &self,
        id: Option<&'a Uuid>,
        group_id: &[u8],
        participants: &[(&'a [u8], u32)],
        threshold: u32,
        name: &str,
        task_data: &[u8],
        request: &[u8],
        task_type: TaskType,
        key_type: KeyType,
        protocol: ProtocolType,
    ) -> Result<Task, PersistenceError>;

    async fn get_task_models(&self, task_ids: &[Uuid]) -> Result<Vec<Task>, PersistenceError>;

    async fn get_tasks(&self) -> Result<Vec<Uuid>, PersistenceError>;

    async fn get_restart_candidates(&self) -> Result<Vec<Uuid>, PersistenceError>;

    async fn get_active_device_tasks(
        &self,
        identifier: &[u8],
    ) -> Result<Vec<Uuid>, PersistenceError>;

    async fn set_task_decision(
        &self,
        task_id: &Uuid,
        device_id: &[u8],
        accept: bool,
    ) -> Result<(), PersistenceError>;

    async fn get_task_decisions(
        &self,
        task_id: &Uuid,
    ) -> Result<HashMap<Vec<u8>, i8>, PersistenceError>;

    async fn set_task_acknowledgement(
        &self,
        task_id: &Uuid,
        device_id: &[u8],
    ) -> Result<(), PersistenceError>;

    async fn get_task_acknowledgements(
        &self,
        task_id: &Uuid,
    ) -> Result<HashSet<Vec<u8>>, PersistenceError>;

    async fn set_task_active_shares(
        &self,
        task_id: &Uuid,
        active_shares: &HashMap<Vec<u8>, u32>,
    ) -> Result<(), PersistenceError>;

    async fn get_task_active_shares(
        &self,
        task_id: &Uuid,
    ) -> Result<HashMap<Vec<u8>, u32>, PersistenceError>;

    async fn set_task_round(&self, task_id: &Uuid, round: u16) -> Result<u16, PersistenceError>;

    async fn increment_task_attempt_count(&self, task_id: &Uuid) -> Result<u32, PersistenceError>;

    async fn set_task_result(
        &self,
        task_id: &Uuid,
        result: &Result<Vec<u8>, String>,
    ) -> Result<(), PersistenceError>;

    async fn set_task_group_certificates_sent(
        &self,
        task_id: &Uuid,
        group_certificates_sent: Option<bool>,
    ) -> Result<(), PersistenceError>;
}
