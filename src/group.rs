use std::cmp::Eq;
use std::collections::HashMap;
use crate::protocols::ProtocolType;
use crate::device::Device;

#[derive(Clone, Eq)]
pub struct Group {
    identifier: Vec<u8>,
    name: String,
    devices: HashMap<Vec<u8>, Device>, // TODO use HashSet-like collection that can refer to its element fields?
    threshold: u32,
    protocol: ProtocolType,
    certificate: Vec<u8>
}

impl Group {
    pub fn new(identifier: Vec<u8>, name: String, devices: Vec<Device>, threshold: u32, protocol: ProtocolType, certificate: Vec<u8>) -> Self {
        Group { identifier, name, devices: devices.into_iter().map(|x| (x.identifier().to_vec(), x)).collect(), threshold, protocol, certificate }
    }

    pub fn identifier(&self) -> &[u8] {
        &self.identifier
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    pub fn reject_threshold(&self) -> u32 {
        self.devices.len() as u32 - self.threshold + 1 // rejects >= threshold_reject => fail
    }

    pub fn devices(&self) -> &HashMap<Vec<u8>, Device> {
        &self.devices
    }

    pub fn contains(&self, device_id: &[u8]) -> bool {
        self.devices.contains_key(device_id)
    }

    pub fn protocol(&self) -> ProtocolType { self.protocol }

    pub fn certificate(&self) -> &[u8] { &self.certificate }
}

impl PartialEq for Group {
    fn eq(&self, other: &Self) -> bool {
        self.identifier == other.identifier
    }
}

impl From<&Group> for crate::proto::Group {
    fn from(group: &Group) -> Self {
        crate::proto::Group {
            identifier: group.identifier().to_vec(),
            name: group.name().to_owned(),
            threshold: group.threshold(),
            device_ids: group.devices().keys().map(Vec::clone).collect(),
        }
    }
}
