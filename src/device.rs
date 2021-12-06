use std::hash::{Hash, Hasher};
use std::borrow::Borrow;

#[derive(Clone, Eq)]
pub struct Device {
    identifier: Vec<u8>,
    // protocol: ProtocolType
}

impl Device {
    pub fn new(identifier: Vec<u8>) -> Self {
        Device { identifier }
    }

    pub fn identifier(&self) -> &[u8] {
        &self.identifier
    }
}

impl PartialEq for Device {
    fn eq(&self, other: &Self) -> bool {
        self.identifier == other.identifier
    }
}

impl Hash for Device {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identifier.hash(state);
    }
}

impl Borrow<[u8]> for Device {
    fn borrow(&self) -> &[u8] {
        &self.identifier
    }
}
