use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Eq, Debug)]
pub struct Device {
    identifier: Vec<u8>,
    name: String,
    last_active: u64,
}

impl Device {
    pub fn new(identifier: Vec<u8>, name: String) -> Self {
        assert!(!identifier.is_empty());
        Device {
            identifier,
            name,
            last_active: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
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
        self.last_active = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        self.last_active
    }
}

impl PartialEq for Device {
    fn eq(&self, other: &Self) -> bool {
        self.identifier == other.identifier
    }
}

impl PartialOrd for Device {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.identifier.partial_cmp(&other.identifier)
    }
}

impl From<&Device> for crate::proto::Device {
    fn from(device: &Device) -> Self {
        crate::proto::Device {
            identifier: device.identifier().to_vec(),
            name: device.name().to_string(),
            last_active: device.last_active(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic]
    fn empty_identifier() {
        Device::new(vec![], String::from("Sample Device"));
    }

    #[test]
    fn compare_devices() {
        let d0 = Device::new(vec![0x00], String::from("d0"));
        let d1 = Device::new(vec![0x01], String::from("d1"));
        let d2 = Device::new(vec![0x02], String::from("d2"));
        let d0_duplicate = Device::new(vec![0x00], String::from("d0_duplicate"));
        assert!(d0 < d1);
        assert!(d1 < d2);
        assert!(d0 < d2);
        assert!(d0 == d0_duplicate);
    }

    #[test]
    fn protobuf_device() {
        let device = Device::new(vec![0x01, 0x02, 0x03, 0x04], String::from("Sample Device"));
        let protobuf = crate::proto::Device::from(&device);
        assert_eq!(protobuf.identifier, device.identifier());
        assert_eq!(protobuf.name, device.name());
        assert_eq!(protobuf.last_active, device.last_active());
    }

    #[test]
    fn sample_device() {
        let identifier = vec![0x01, 0x02, 0x03, 0x04];
        let name = String::from("Sample Device");
        let mut device = Device::new(identifier.clone(), name.clone());
        assert_eq!(device.identifier(), &identifier);
        assert_eq!(device.name(), &name);
        let previous_active = device.last_active();
        let activated = device.activated();
        assert!(previous_active <= device.last_active());
        assert_eq!(device.last_active(), activated);
    }
}
