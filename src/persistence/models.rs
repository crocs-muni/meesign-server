use diesel::{query_builder::AsChangeset, Insertable, Queryable, Selectable};
use serde::Serialize;
use uuid::Uuid;

use crate::persistence::schema::*;

use super::{
    enums::{DeviceKind, KeyType, ProtocolType, TaskState, TaskType},
    PersistenceError,
};

#[derive(Insertable)]
#[diesel(table_name = device)]
pub struct NewDevice<'a> {
    pub id: &'a Vec<u8>,
    pub name: &'a str,
    pub kind: &'a DeviceKind,
    pub certificate: &'a Vec<u8>,
}

#[derive(Clone)]
pub struct Participant {
    pub device: Device,
    pub shares: u32,
}

#[derive(Queryable, Selectable, Clone, PartialEq, Eq)]
#[diesel(table_name = device)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Device {
    pub id: Vec<u8>,
    pub name: String,
    pub kind: DeviceKind,
    pub certificate: Vec<u8>,
}

impl Device {
    #[cfg(test)]
    pub fn new(id: Vec<u8>, name: String, kind: DeviceKind, certificate: Vec<u8>) -> Self {
        Self {
            id,
            name,
            kind,
            certificate,
        }
    }
    pub fn identifier(&self) -> &Vec<u8> {
        &self.id
    }
}

#[derive(Queryable, Clone, Eq, PartialEq, Serialize)]
#[cfg_attr(test, derive(PartialOrd, Ord, Debug))]
pub struct Group {
    pub id: Vec<u8>,
    pub name: String,
    pub threshold: i32,
    pub protocol: ProtocolType,
    pub key_type: KeyType,
    pub certificate: Option<Vec<u8>>,
    pub note: Option<String>,
    #[serde(flatten)]
    pub participant_ids_shares: Vec<(Vec<u8>, u32)>,
}

#[derive(Insertable)]
#[diesel(table_name=group)]
pub struct NewGroup<'a> {
    pub id: &'a [u8],
    pub name: &'a str,
    pub threshold: i32,
    pub protocol: ProtocolType,
    pub key_type: KeyType,
    pub certificate: Option<&'a [u8]>,
    pub note: Option<&'a str>,
}

#[derive(Insertable)]
#[diesel(table_name=group_participant)]
pub struct NewGroupParticipant<'a> {
    pub device_id: &'a [u8],
    pub group_id: &'a [u8],
    pub shares: i32,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = group_participant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct GroupParticipant {
    pub device_id: Vec<u8>,
    pub group_id: Vec<u8>,
    pub shares: i32,
}

#[derive(Insertable)]
#[diesel(table_name=task_participant)]
pub struct NewTaskParticipant<'a> {
    pub device_id: &'a [u8],
    pub task_id: &'a Uuid,
    pub shares: i32,
    pub decision: Option<bool>,
    pub acknowledgment: Option<bool>,
}

#[derive(Queryable, Serialize, Clone, Eq, PartialEq)]
pub struct Task {
    pub id: Uuid,
    pub protocol_round: i32,
    pub attempt_count: i32,
    pub threshold: i32,
    pub task_data: Option<Vec<u8>>,
    pub preprocessed: Option<Vec<u8>>,
    pub request: Vec<u8>,
    pub group_id: Option<Vec<u8>>,
    pub task_type: TaskType,
    pub task_state: TaskState,
    pub key_type: KeyType,
    pub protocol_type: ProtocolType,
    pub note: Option<String>,
    pub group_certificates_sent: Option<bool>,
    #[serde(flatten)]
    pub result: Option<TaskResult>,
}

#[derive(Insertable)]
#[diesel(table_name=task)]
pub struct NewTask<'a> {
    pub id: Option<&'a Uuid>,
    pub protocol_round: i32,
    pub attempt_count: i32,
    pub threshold: i32,
    pub task_data: Option<&'a [u8]>,
    pub preprocessed: Option<&'a [u8]>,
    pub request: &'a [u8],
    pub task_type: TaskType,
    pub task_state: TaskState,
    pub group_id: Option<&'a [u8]>,
    pub key_type: KeyType,
    pub protocol_type: ProtocolType,
    pub note: Option<&'a str>,
    pub group_certificates_sent: Option<bool>,
}

#[derive(Queryable, Selectable, Serialize, Clone, Eq, PartialEq, Debug)]
#[diesel(check_for_backend(diesel::pg::Pg))]
#[diesel(table_name=task_result)]
pub struct TaskResult {
    pub task_id: Uuid,
    pub is_successful: bool,
    pub data: Option<Vec<u8>>,
    pub error_message: Option<String>,
}

impl TaskResult {
    pub fn try_into_option(self) -> Result<Option<Result<Vec<u8>, String>>, PersistenceError> {
        if self.is_successful && self.data.is_some() {
            Ok(Some(Ok(self.data.unwrap())))
        } else if !self.is_successful && self.error_message.is_some() {
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
    pub is_successful: bool,
    pub data: Option<&'a [u8]>,
    pub error_message: Option<&'a str>,
}

impl<'a> NewTaskResult<'a> {
    pub fn from(result: &'a Result<Vec<u8>, String>, task_id: &'a Uuid) -> Self {
        match result {
            Ok(data) => Self {
                task_id,
                is_successful: true,
                data: Some(data.as_slice()),
                error_message: None,
            },
            Err(error_message) => Self {
                task_id,
                is_successful: false,
                data: None,
                error_message: Some(error_message),
            },
        }
    }
}
