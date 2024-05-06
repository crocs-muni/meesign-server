use chrono::{DateTime, Local};
use diesel::{
    associations::{Associations, Identifiable},
    query_builder::AsChangeset,
    Insertable, Queryable, Selectable,
};
use uuid::Uuid;

use crate::persistence::schema::*;

use super::{
    enums::{KeyType, ProtocolType, TaskState, TaskType},
    PersistenceError,
};

#[derive(Insertable)]
#[diesel(table_name = device)]
pub struct NewDevice<'a> {
    pub id: &'a Vec<u8>,
    pub name: &'a str,
    pub certificate: &'a Vec<u8>,
}

#[derive(Queryable, Selectable, Clone)]
#[diesel(table_name = device)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Device {
    pub id: Vec<u8>,
    pub name: String,
    pub certificate: Vec<u8>,
    pub last_active: DateTime<Local>,
}

impl Device {
    #[cfg(test)]
    pub fn new(id: Vec<u8>, device_name: String, device_certificate: Vec<u8>) -> Self {
        Self {
            id,
            name: device_name,
            certificate: device_certificate,
            last_active: Local::now(),
        }
    }
    pub fn identifier(&self) -> &Vec<u8> {
        &self.id
    }

    pub fn last_active(&self) -> &DateTime<Local> {
        &self.last_active
    }
}

impl From<Device> for crate::proto::Device {
    fn from(device: Device) -> Self {
        crate::proto::Device {
            identifier: device.id,
            name: device.name,
            certificate: device.certificate,
            last_active: device.last_active.timestamp() as u64,
        }
    }
}

#[derive(Queryable, Clone, Eq, PartialEq, Selectable)]
#[cfg_attr(test, derive(PartialOrd, Ord, Debug))]
#[diesel(table_name=group)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Group {
    pub id: i32,
    pub identifier: Vec<u8>,
    pub name: String,
    pub threshold: i32,
    pub protocol: ProtocolType,
    pub round: i32,
    pub key_type: KeyType,
    pub certificate: Option<Vec<u8>>,
}

#[derive(Insertable)]
#[diesel(table_name=group)]
pub struct NewGroup<'a> {
    pub identifier: &'a [u8],
    pub name: &'a str,
    pub threshold: i32,
    pub protocol: ProtocolType,
    pub round: i32,
    pub key_type: KeyType,
    pub certificate: Option<&'a [u8]>,
}

#[derive(Insertable)]
#[diesel(table_name=group_participant)]
pub struct NewGroupParticipant<'a> {
    pub device_id: &'a [u8],
    pub group_id: i32,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = group_participant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct GroupParticipant {
    pub id: i32,
    pub device_id: Vec<u8>,
    pub group_id: i32,
}

#[derive(Insertable)]
#[diesel(table_name=task_participant)]
pub struct NewTaskParticipant<'a> {
    pub device_id: &'a [u8],
    pub task_id: &'a Uuid,
    pub decision: Option<bool>,
    pub acknowledgment: Option<bool>,
}

#[derive(Queryable, Selectable, Clone, Eq, PartialEq)]
#[diesel(table_name=task)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PartialTask {
    pub id: Uuid,
    pub protocol_round: i32,
    pub attempt_count: i32,
    pub error_message: Option<String>,
    pub threshold: i32,
    pub last_update: DateTime<Local>,
    pub task_data: Option<Vec<u8>>,
    pub preprocessed: Option<Vec<u8>>,
    pub request: Option<Vec<u8>>,
    pub group_id: Option<i32>,
    pub task_type: TaskType,
    pub task_state: TaskState,
    pub key_type: Option<KeyType>,
    pub protocol_type: Option<ProtocolType>,
}

// TODO: attempt to write a diesel query that will serialize this.
// This is an inefficient hotfix
pub struct Task {
    pub id: Uuid,
    pub protocol_round: i32,
    pub attempt_count: i32,
    pub error_message: Option<String>,
    pub threshold: i32,
    pub last_update: DateTime<Local>,
    pub task_data: Option<Vec<u8>>,
    pub preprocessed: Option<Vec<u8>>,
    pub request: Option<Vec<u8>>,
    pub group_id: Option<i32>,
    pub task_type: TaskType,
    pub task_state: TaskState,
    pub key_type: Option<KeyType>,
    pub protocol_type: Option<ProtocolType>,
    pub result: Option<Result<Vec<u8>, String>>,
}
impl Task {
    pub fn try_from(
        task: PartialTask,
        result: Option<TaskResult>,
    ) -> Result<Self, PersistenceError> {
        let result = match result.map(|result| result.try_into_option()) {
            Some(val) => val?,
            None => None,
        };
        Ok(Self {
            id: task.id,
            protocol_round: task.protocol_round,
            attempt_count: task.attempt_count,
            error_message: task.error_message,
            threshold: task.threshold,
            last_update: task.last_update,
            task_data: task.task_data,
            preprocessed: task.preprocessed,
            request: task.request,
            group_id: task.group_id,
            task_type: task.task_type,
            task_state: task.task_state,
            key_type: task.key_type,
            protocol_type: task.protocol_type,
            result,
        })
    }
}

#[derive(Insertable)]
#[diesel(table_name=task)]
pub struct NewTask<'a> {
    pub id: Option<&'a Uuid>,
    pub protocol_round: i32,
    pub attempt_count: i32,
    pub error_message: Option<&'a str>,
    pub threshold: i32,
    pub last_update: Option<DateTime<Local>>,
    pub task_data: Option<&'a [u8]>,
    pub preprocessed: Option<&'a [u8]>,
    pub request: Option<&'a [u8]>,
    pub task_type: TaskType,
    pub task_state: TaskState,
    pub key_type: Option<KeyType>,
    pub protocol_type: Option<ProtocolType>,
}

#[derive(Queryable, Clone, Eq, PartialEq, Selectable, Debug)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(table_name=task_result)]
pub struct TaskResult {
    pub id: i32,
    pub task_id: Uuid,
    pub is_successfull: bool,
    pub data: Option<Vec<u8>>,
    pub error_message: Option<String>,
}

impl TaskResult {
    pub fn try_into_option(self) -> Result<Option<Result<Vec<u8>, String>>, PersistenceError> {
        if self.is_successfull && self.data.is_some() {
            Ok(Some(Ok(self.data.unwrap())))
        } else if !self.is_successfull && self.error_message.is_some() {
            Ok(Some(Err(self.error_message.unwrap())))
        } else {
            Err(PersistenceError::DataInconsistencyError(format!(
                "Inconsistent task result: {:?}",
                self
            )))
        }
    }
}

#[derive(Insertable, AsChangeset)]
#[diesel(table_name=task_result)]
pub struct NewTaskResult<'a> {
    pub task_id: &'a Uuid,
    pub is_successfull: bool,
    pub data: Option<&'a [u8]>,
    pub error_message: Option<&'a str>,
}

impl<'a> NewTaskResult<'a> {
    pub fn from(result: &'a Result<Vec<u8>, String>, task_id: &'a Uuid) -> Self {
        match result {
            Ok(data) => Self {
                task_id,
                is_successfull: true,
                data: Some(data.as_slice()),
                error_message: None,
            },
            Err(error_message) => Self {
                task_id,
                is_successfull: false,
                data: None,
                error_message: Some(error_message),
            },
        }
    }
}

impl From<Group> for crate::group::Group {
    fn from(value: Group) -> Self {
        Self::new(
            value.identifier,
            value.name,
            value.threshold as u32,
            value.protocol.into(),
            value.key_type.into(),
            value.certificate,
        )
    }
}
