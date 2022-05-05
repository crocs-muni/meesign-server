use std::cmp::Eq;
use std::hash::{Hash, Hasher};
use std::borrow::Borrow;
use std::collections::HashSet;
use crate::protocols::ProtocolType;
use prost::Message;

#[derive(Clone, Eq)]
pub struct Group {
    identifier: Vec<u8>,
    name: String,
    devices: HashSet<Vec<u8>>,
    threshold: u32,
    protocol: ProtocolType,
    certificate: Vec<u8>
}

impl Group {
    pub fn new(identifier: Vec<u8>, name: String, devices: Vec<Vec<u8>>, threshold: u32, protocol: ProtocolType, certificate: Vec<u8>) -> Self {
        Group { identifier, name, devices: devices.iter().map(Vec::clone).collect(), threshold, protocol, certificate }
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

    pub fn devices(&self) -> &HashSet<Vec<u8>> {
        &self.devices
    }

    pub fn contains(&self, device: &Vec<u8>) -> bool {
        self.devices.contains(device)
    }

    pub fn protocol(&self) -> ProtocolType { self.protocol }

    pub fn certificate(&self) -> &[u8] { &self.certificate }

    pub fn encode(&self) -> Vec<u8> {
        (crate::proto::Group {
            id: self.identifier().to_vec(),
            name: self.name().to_string(),
            threshold: self.threshold(),
            device_ids: self.devices().iter().map(Vec::clone).collect()
        }).encode_to_vec()
    }
}

impl PartialEq for Group {
    fn eq(&self, other: &Self) -> bool {
        self.identifier == other.identifier
    }
}

impl Hash for Group {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identifier.hash(state);
    }
}

impl Borrow<[u8]> for Group {
    fn borrow(&self) -> &[u8] {
        &self.identifier
    }
}
