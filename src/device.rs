use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct Device {
    identifier: Vec<u8>,
    name: String,
    certificate: Vec<u8>,
    last_active: AtomicU64,
    admin: bool,
}

impl Device {
    pub fn new(identifier: Vec<u8>, name: String, certificate: Vec<u8>) -> Self {
        assert!(!identifier.is_empty());
        assert!(!certificate.is_empty());
        Device {
            identifier,
            name,
            certificate,
            last_active: AtomicU64::new(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            ),
            admin: false,
        }
    }

    pub fn identifier(&self) -> &[u8] {
        &self.identifier
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn certificate(&self) -> &[u8] {
        &self.certificate
    }

    pub fn admin(&self) -> bool {
        self.admin
    }

    pub fn set_admin(&mut self, admin: bool) {
        self.admin = admin;
    }

    pub fn last_active(&self) -> u64 {
        self.last_active.load(Ordering::Relaxed)
    }

    pub fn activated(&self) -> u64 {
        self.last_active.store(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            Ordering::Relaxed,
        );
        self.last_active.load(Ordering::Relaxed)
    }
}

impl From<&Device> for crate::proto::Device {
    fn from(device: &Device) -> Self {
        crate::proto::Device {
            identifier: device.identifier().to_vec(),
            name: device.name().to_string(),
            certificate: device.certificate().to_vec(),
            admin: device.admin(),
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
        Device::new(vec![], String::from("Sample Device"), vec![0xff]);
    }

    #[test]
    #[should_panic]
    fn empty_certificate() {
        Device::new(vec![0xff], String::from("Sample Device"), vec![]);
    }

    #[test]
    fn protobuf_device() {
        let device = Device::new(
            vec![0x01, 0x02, 0x03, 0x04],
            String::from("Sample Device"),
            vec![0xab, 0xcd, 0xef, 0x00],
        );
        let protobuf = crate::proto::Device::from(&device);
        assert_eq!(protobuf.identifier, device.identifier());
        assert_eq!(protobuf.name, device.name());
        assert_eq!(protobuf.certificate, device.certificate());
        assert_eq!(protobuf.last_active, device.last_active());
    }

    #[test]
    fn sample_device() {
        let identifier = vec![0x01, 0x02, 0x03, 0x04];
        let name = String::from("Sample Device");
        let certificate = vec![0xab, 0xcd, 0xef, 0x00];
        let device = Device::new(identifier.clone(), name.clone(), certificate.clone());
        assert_eq!(device.identifier(), &identifier);
        assert_eq!(device.name(), &name);
        assert_eq!(device.certificate(), &certificate);
        let previous_active = device.last_active();
        let activated = device.activated();
        assert!(previous_active <= device.last_active());
        assert_eq!(device.last_active(), activated);
    }

    #[test]
    fn sample_device_admin() {
        let identifier = vec![0x01, 0x02, 0x03, 0x04];
        let name = String::from("Sample Device");
        let certificate = vec![0xab, 0xcd, 0xef, 0x00];
        let mut device = Device::new(identifier.clone(), name.clone(), certificate.clone());
        assert!(!device.admin());
        device.set_admin(true);
        assert!(device.admin());
    }
}
