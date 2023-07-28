use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub struct Device {
    identifier: Vec<u8>,
    name: String,
    certificate: Vec<u8>,
    last_active: AtomicU64,
    role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    None = 0,
    User = 1,
    Operator = 2,
    Admin = 3,
}

impl Role {
    pub fn from_u32(role: u32) -> Self {
        match role {
            0 => Role::None,
            1 => Role::User,
            2 => Role::Operator,
            3 => Role::Admin,
            _ => panic!("Invalid role {}", role),
        }
    }
}

impl Device {
    pub fn new(identifier: Vec<u8>, name: String, certificate: Vec<u8>, role: Role) -> Self {
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
            role: role,
        }
    }

    pub fn identifier(&self) -> &[u8] {
        &self.identifier
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name
    }

    pub fn certificate(&self) -> &[u8] {
        &self.certificate
    }

    pub fn role(&self) -> Role {
        self.role
    }

    pub fn set_role(&mut self, role: Role) {
        self.role = role;
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
            role: device.role() as u32,
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
        Device::new(
            vec![],
            String::from("Sample Device"),
            vec![0xff],
            Role::User,
        );
    }

    #[test]
    #[should_panic]
    fn empty_certificate() {
        Device::new(
            vec![0xff],
            String::from("Sample Device"),
            vec![],
            Role::User,
        );
    }

    #[test]
    fn protobuf_device() {
        let device = Device::new(
            vec![0x01, 0x02, 0x03, 0x04],
            String::from("Sample Device"),
            vec![0xab, 0xcd, 0xef, 0x00],
            Role::User,
        );
        let protobuf = crate::proto::Device::from(&device);
        assert_eq!(protobuf.identifier, device.identifier());
        assert_eq!(protobuf.name, device.name());
        assert_eq!(protobuf.certificate, device.certificate());
        assert_eq!(protobuf.last_active, device.last_active());
        assert_eq!(protobuf.role, device.role() as u32);
    }

    #[test]
    fn sample_device() {
        let identifier = vec![0x01, 0x02, 0x03, 0x04];
        let name = String::from("Sample Device");
        let certificate = vec![0xab, 0xcd, 0xef, 0x00];
        let role = Role::User;
        let device = Device::new(identifier.clone(), name.clone(), certificate.clone(), role);
        assert_eq!(device.identifier(), &identifier);
        assert_eq!(device.name(), &name);
        assert_eq!(device.certificate(), &certificate);
        let previous_active = device.last_active();
        let activated = device.activated();
        assert!(previous_active <= device.last_active());
        assert_eq!(device.last_active(), activated);
        assert_eq!(device.role(), role);
    }

    #[test]
    fn sample_device_admin() {
        let identifier = vec![0x01, 0x02, 0x03, 0x04];
        let name = String::from("Sample Device");
        let certificate = vec![0xab, 0xcd, 0xef, 0x00];
        let mut device = Device::new(
            identifier.clone(),
            name.clone(),
            certificate.clone(),
            Role::User,
        );
        assert!(device.role() == Role::User);
        device.set_role(Role::Admin);
        assert!(device.role() == Role::Admin);
    }
}
