use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Eq)]
pub struct Device {
    identifier: Vec<u8>,
    name: String,
    last_active: u64,
    // protocol: ProtocolType
}

impl Device {
    pub fn new(identifier: Vec<u8>, name: String) -> Self {
        Device {
            identifier,
            name,
            last_active: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
        }
    }

    pub fn identifier(&self) -> &[u8] {
        &self.identifier
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn last_active(&self) -> u64 {
        self.last_active
    }

    pub fn activated(&mut self) -> u64 {
        self.last_active = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        self.last_active
    }
}

impl PartialEq for Device {
    fn eq(&self, other: &Self) -> bool {
        self.identifier == other.identifier
    }
}


impl From<&Device> for crate::proto::Device {
    fn from(device: &Device) -> Self {
        crate::proto::Device {
            identifier: device.identifier().to_vec(),
            name: device.name().to_string(),
            last_active: device.last_active()
        }
    }
}
