use crate::error::Error;
use crate::persistence::{Device, Group as GroupModel};
use crate::proto::{KeyType, ProtocolType};
#[derive(Clone)]
pub struct Group {
    identifier: Vec<u8>,
    name: String,
    devices: Vec<Device>,
    threshold: u32,
    protocol: ProtocolType,
    key_type: KeyType,
    certificate: Option<Vec<u8>>,
    note: Option<String>,
}

impl Group {
    pub fn new(
        identifier: Vec<u8>,
        name: String,
        threshold: u32,
        devices: Vec<Device>,
        protocol: ProtocolType,
        key_type: KeyType,
        certificate: Option<Vec<u8>>,
        note: Option<String>,
    ) -> Self {
        assert!(!identifier.is_empty());
        assert!(threshold >= 1);
        Group {
            identifier,
            name,
            devices,
            threshold,
            protocol,
            key_type,
            certificate,
            note,
        }
    }

    pub fn identifier(&self) -> &[u8] {
        &self.identifier
    }

    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn devices(&self) -> &Vec<Device> {
        &self.devices
    }
    pub fn threshold(&self) -> u32 {
        self.threshold
    }

    pub fn reject_threshold(&self) -> u32 {
        self.devices.len() as u32 - self.threshold + 1 // rejects >= threshold_reject => fail
    }

    pub fn protocol(&self) -> ProtocolType {
        self.protocol
    }

    pub fn key_type(&self) -> KeyType {
        self.key_type
    }

    pub fn certificate(&self) -> Option<&Vec<u8>> {
        self.certificate.as_ref()
    }

    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    // TODO: consider merging Group with GroupModel
    pub fn try_from_model(value: GroupModel, devices: Vec<Device>) -> Result<Self, Error> {
        Ok(Self {
            identifier: value.id,
            name: value.name,
            threshold: value.threshold as u32,
            devices,
            protocol: value.protocol.into(),
            key_type: value.key_type.into(),
            certificate: value.certificate,
            note: value.note,
        })
    }
}

// impl From<&Group> for crate::proto::Group {
//     fn from(group: &Group) -> Self {
//         crate::proto::Group {
//             identifier: group.identifier().to_vec(),
//             name: group.name().to_owned(),
//             threshold: group.threshold(),
//             device_ids: group
//                 .devices()
//                 .iter()
//                 .map(|x| x.identifier())
//                 .map(Vec::from)
//                 .collect(),
//             protocol: group.protocol().into(),
//             key_type: group.key_type().into(),
//             note: group.note().map(String::from),
//         }
//     }
// }

#[cfg(test)]
mod tests {
    use crate::persistence::DeviceKind;
    use std::vec;

    use super::*;

    #[test]
    #[should_panic]
    fn empty_identifier() {
        Group::new(
            vec![],
            String::from("Sample Group"),
            2,
            vec![],
            ProtocolType::Gg18,
            KeyType::SignPdf,
            None,
            None,
        );
    }

    // #[test]
    // fn protobuf_group() {
    //     let group = Group::new(
    //         vec![0x00],
    //         String::from("Sample Group"),
    //         prepare_devices(3),
    //         2,
    //         ProtocolType::Gg18,
    //         KeyType::SignPdf,
    //         None,
    //         None,
    //     );
    //     let protobuf = crate::proto::Group::from(&group);
    //     assert_eq!(protobuf.identifier, group.identifier());
    //     assert_eq!(protobuf.name, group.name());
    //     assert_eq!(protobuf.threshold, group.threshold());
    //     assert_eq!(
    //         protobuf.device_ids,
    //         group
    //             .devices()
    //             .iter()
    //             .map(|device| device.identifier())
    //             .map(Vec::from)
    //             .collect::<Vec<_>>()
    //     );
    //     assert_eq!(protobuf.protocol, group.protocol() as i32);
    //     assert_eq!(protobuf.key_type, group.key_type() as i32);
    // }

    #[test]
    fn sample_group() {
        let identifier = vec![0x01, 0x02, 0x03, 0x04];
        let name = String::from("Sample Group");
        let mut devices = prepare_devices(6);
        let extra_device = devices.pop().unwrap();
        let threshold = 3;
        let protocol_type = ProtocolType::Gg18;
        let key_type = KeyType::SignPdf;
        let group = Group::new(
            identifier.clone(),
            name.clone(),
            threshold,
            devices.clone(),
            protocol_type,
            key_type,
            None,
            Some("time policy".into()),
        );
        assert_eq!(group.identifier(), &identifier);
        assert_eq!(group.name(), &name);
        assert_eq!(group.threshold(), threshold);
        assert_eq!(group.reject_threshold(), 3);
        for (left_device, right_device) in group.devices().iter().zip(devices.iter()) {
            assert_eq!(left_device.identifier(), right_device.identifier());
        }
        assert!(!group
            .devices()
            .iter()
            .any(|dev| dev.identifier() == extra_device.identifier()));
        assert_eq!(group.protocol(), protocol_type.into());
        assert_eq!(group.key_type(), key_type.into());
        assert_eq!(group.certificate(), None);
    }

    fn prepare_devices(n: usize) -> Vec<Device> {
        assert!(n < u8::MAX as usize);
        (0..n)
            .map(|i| {
                Device::new(
                    vec![i as u8],
                    format!("d{}", i),
                    DeviceKind::User,
                    vec![0xf0 | i as u8],
                )
            })
            .collect()
    }
}
