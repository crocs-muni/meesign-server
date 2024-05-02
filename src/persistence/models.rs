use chrono::{DateTime, Local};
use diesel::{Insertable, Queryable, Selectable};
use uuid::Uuid;

use crate::persistence::schema::*;

use super::enums::{KeyType, ProtocolType, TaskState, TaskType};

#[derive(Insertable)]
#[diesel(table_name = device)]
pub struct NewDevice<'a> {
    pub id: &'a Vec<u8>,
    pub device_name: &'a str,
    pub device_certificate: &'a Vec<u8>,
}

#[derive(Queryable, Selectable, Clone)]
#[diesel(table_name = device)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Device {
    pub id: Vec<u8>,
    pub device_name: String,
    pub device_certificate: Vec<u8>,
    pub last_active: DateTime<Local>,
}

impl Device {
    #[cfg(test)]
    pub fn new(id: Vec<u8>, device_name: String, device_certificate: Vec<u8>) -> Self {
        Self {
            id,
            device_name,
            device_certificate,
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
            name: device.device_name,
            certificate: device.device_certificate,
            last_active: device.last_active.timestamp_millis() as u64,
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
    pub group_name: String,
    pub threshold: i32,
    pub protocol: ProtocolType,
    pub round: i32,
    pub key_type: KeyType,
    pub group_certificate: Option<Vec<u8>>,
}

#[derive(Insertable)]
#[diesel(table_name=group)]
pub struct NewGroup<'a> {
    pub identifier: &'a [u8],
    pub group_name: &'a str,
    pub threshold: i32,
    pub protocol: ProtocolType,
    pub round: i32,
    pub key_type: KeyType,
    pub group_certificate: Option<&'a [u8]>,
}

#[derive(Insertable)]
#[diesel(table_name=groupparticipant)]
pub struct NewGroupParticipant<'a> {
    pub device_id: &'a [u8],
    pub group_id: i32,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = groupparticipant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct GroupParticipant {
    pub id: i32,
    pub device_id: Vec<u8>,
    pub group_id: i32,
}

#[derive(Insertable)]
#[diesel(table_name=taskparticipant)]
pub struct NewTaskParticipant<'a> {
    pub device_id: &'a [u8],
    pub task_id: &'a Uuid,
    pub decision: Option<bool>,
    pub acknowledgment: Option<bool>,
}

#[derive(Queryable, Clone, Eq, PartialEq, Selectable)]
#[diesel(table_name=task)]
#[diesel(check_for_backend(diesel::pg::Pg))]
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
}

#[derive(Insertable)]
#[diesel(table_name=task)]
pub struct NewTask<'a> {
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

impl From<Group> for crate::group::Group {
    fn from(value: Group) -> Self {
        Self::new(
            value.identifier,
            value.group_name,
            value.threshold as u32,
            value.protocol.into(),
            value.key_type.into(),
            value.group_certificate,
        )
    }
}
