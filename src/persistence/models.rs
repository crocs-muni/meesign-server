use chrono::NaiveDateTime;
use diesel::{Insertable, Queryable, Selectable};

use crate::persistence::enums::ProtocolType;
use crate::persistence::schema::*;

use super::enums::KeyType;

#[derive(Insertable)]
#[diesel(table_name = device)]
pub struct NewDevice<'a> {
    pub identifier: &'a Vec<u8>,
    pub device_name: &'a str,
    pub device_certificate: &'a Vec<u8>,
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = device)]
pub struct Device {
    pub id: i32,
    pub identifier: Vec<u8>,
    pub device_name: String,
    pub device_certificate: Vec<u8>,
    pub last_active: NaiveDateTime,
}

impl From<&Device> for crate::proto::Device {
    fn from(device: &Device) -> Self {
        crate::proto::Device {
            identifier: device.identifier.to_vec(),
            name: device.device_name.to_string(),
            certificate: device.device_certificate.to_vec(),
            last_active: device.last_active.timestamp_millis() as u64,
        }
    }
}

#[derive(Queryable, Clone, Eq, PartialEq, Selectable)]
#[diesel(table_name=signinggroup)]
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
#[diesel(table_name=signinggroup)]
pub struct NewGroup<'a> {
    pub identifier: &'a [u8],
    pub group_name: &'a str,
    pub threshold: i32,
    pub protocol: ProtocolType,
    pub round: i32,
    pub group_certificate: Option<&'a [u8]>,
}

#[derive(Insertable)]
#[diesel(table_name=groupparticipant)]
pub struct NewGroupParticipant {
    pub device_id: i32,
    pub group_id: i32,
}
