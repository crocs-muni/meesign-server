#[derive(Clone, Eq)]
pub struct Device {
    identifier: Vec<u8>,
    name: String,
    // protocol: ProtocolType
}

impl Device {
    pub fn new(identifier: Vec<u8>, name: String) -> Self {
        Device { identifier, name }
    }

    pub fn identifier(&self) -> &[u8] {
        &self.identifier
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl PartialEq for Device {
    fn eq(&self, other: &Self) -> bool {
        self.identifier == other.identifier
    }
}
