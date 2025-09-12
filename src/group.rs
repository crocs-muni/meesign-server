use crate::persistence::{Group as GroupModel, Participant};
use crate::proto::{KeyType, ProtocolType};
#[derive(Clone)]
pub struct Group {
    identifier: Vec<u8>,
    name: String,
    participants: Vec<Participant>,
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
        participants: Vec<Participant>,
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
            participants,
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
    pub fn participants(&self) -> &Vec<Participant> {
        &self.participants
    }
    pub fn threshold(&self) -> u32 {
        self.threshold
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
    pub fn from_model(value: GroupModel, participants: Vec<Participant>) -> Self {
        Self {
            identifier: value.id,
            name: value.name,
            threshold: value.threshold as u32,
            participants,
            protocol: value.protocol.into(),
            key_type: value.key_type.into(),
            certificate: value.certificate,
            note: value.note,
        }
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
    use crate::persistence::{Device, DeviceKind};
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
        let mut participants = prepare_participants(6);
        let extra_participant = participants.pop().unwrap();
        let threshold = 3;
        let protocol_type = ProtocolType::Gg18;
        let key_type = KeyType::SignPdf;
        let group = Group::new(
            identifier.clone(),
            name.clone(),
            threshold,
            participants.clone(),
            protocol_type,
            key_type,
            None,
            Some("time policy".into()),
        );
        assert_eq!(group.identifier(), &identifier);
        assert_eq!(group.name(), &name);
        assert_eq!(group.threshold(), threshold);
        for (a, b) in group.participants().iter().zip(participants.iter()) {
            assert_eq!(a.device.identifier(), b.device.identifier());
        }
        assert!(!group
            .participants()
            .iter()
            .any(|p| p.device.identifier() == extra_participant.device.identifier()));
        assert_eq!(group.protocol(), protocol_type.into());
        assert_eq!(group.key_type(), key_type.into());
        assert_eq!(group.certificate(), None);
    }

    fn prepare_participants(n: usize) -> Vec<Participant> {
        assert!(n < u8::MAX as usize);
        (0..n)
            .map(|i| {
                let device = Device::new(
                    vec![i as u8],
                    format!("d{}", i),
                    DeviceKind::User,
                    vec![0xf0 | i as u8],
                );
                Participant { device, shares: 1 }
            })
            .collect()
    }
}
