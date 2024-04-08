use crate::proto::DeviceKind;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct Device {
    identifier: Vec<u8>,
    name: String,
    kind: DeviceKind,
    certificate: Vec<u8>,
    last_active: AtomicU64,
}

impl Device {
    pub fn new(identifier: Vec<u8>, name: String, kind: DeviceKind, certificate: Vec<u8>) -> Self {
        assert!(!identifier.is_empty());
        assert!(!certificate.is_empty());
        Device {
            identifier,
            name,
            kind,
            certificate,
            last_active: AtomicU64::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            ),
        }
    }

    pub fn identifier(&self) -> &[u8] {
        &self.identifier
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> &DeviceKind {
        &self.kind
    }

    pub fn certificate(&self) -> &[u8] {
        &self.certificate
    }

    pub fn last_active(&self) -> u64 {
        self.last_active.load(Ordering::Relaxed)
    }
}

impl From<&Device> for crate::proto::Device {
    fn from(device: &Device) -> Self {
        crate::proto::Device {
            identifier: device.identifier().to_vec(),
            name: device.name().to_string(),
            kind: *device.kind() as i32,
            certificate: device.certificate().to_vec(),
            last_active: device.last_active(),
        }
    }
}
