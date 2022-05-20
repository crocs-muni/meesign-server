use std::cmp::Eq;
use std::collections::HashMap;
use crate::protocols::ProtocolType;
use prost::Message;
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

    pub fn devices(&self) -> &HashMap<Vec<u8>, Device> {
        &self.devices
    }

    pub fn contains(&self, device_id: &[u8]) -> bool {
        self.devices.contains_key(device_id)
    }

    pub fn protocol(&self) -> ProtocolType { self.protocol }

    pub fn certificate(&self) -> &[u8] { &self.certificate }

    pub fn encode(&self) -> Vec<u8> {
        (crate::proto::Group {
            id: self.identifier().to_vec(),
            name: self.name().to_string(),
            threshold: self.threshold(),
            device_ids: self.devices().keys().map(Vec::clone).collect()
        }).encode_to_vec()
    }
}

impl PartialEq for Group {
    fn eq(&self, other: &Self) -> bool {
        self.identifier == other.identifier
    }
}
