use crate::proto;
use diesel_derive_enum::DbEnum;
use serde::Serialize;

#[derive(Copy, Clone, PartialEq, Eq, Debug, DbEnum, Serialize)]
#[cfg_attr(test, derive(PartialOrd, Ord))]
#[ExistingTypePath = "crate::persistence::schema::sql_types::ProtocolType"]
#[DbValueStyle = "PascalCase"]
pub enum ProtocolType {
    Gg18,
    ElGamal,
    Frost,
    Musig2,
}

impl From<proto::ProtocolType> for ProtocolType {
    fn from(value: proto::ProtocolType) -> Self {
        match value {
            proto::ProtocolType::Gg18 => Self::Gg18,
            proto::ProtocolType::Elgamal => Self::ElGamal,
            proto::ProtocolType::Frost => Self::Frost,
            proto::ProtocolType::Musig2 => Self::Musig2,
        }
    }
}

#[derive(Debug, DbEnum, Clone, PartialEq, Eq, Serialize)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::TaskType"]
#[DbValueStyle = "PascalCase"]
pub enum TaskType {
    Group,
    SignPdf,
    SignChallenge,
    Decrypt,
}

// #[derive(Debug, DbEnum)]
// #[ExistingTypePath = "crate::persistence::schema::sql_types::TaskResultType"]
// #[DbValueStyle = "PascalCase"]
// pub enum TaskResultType {
//     GroupEstablished,
//     Signed,
//     SignedPdf,
//     Decrypted,
// }

#[derive(Debug, DbEnum, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::DeviceKind"]
#[DbValueStyle = "PascalCase"]
pub enum DeviceKind {
    User,
    Bot,
}

#[derive(Debug, DbEnum, Clone, PartialEq, Eq, Serialize)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::TaskState"]
#[DbValueStyle = "PascalCase"]
pub enum TaskState {
    Created,
    Running,
    Finished,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, DbEnum, Serialize)]
#[cfg_attr(test, derive(PartialOrd, Ord))]
#[ExistingTypePath = "crate::persistence::schema::sql_types::KeyType"]
#[DbValueStyle = "PascalCase"]
pub enum KeyType {
    SignPdf,
    SignChallenge,
    Decrypt,
}

impl From<proto::KeyType> for KeyType {
    fn from(value: proto::KeyType) -> Self {
        match value {
            proto::KeyType::SignPdf => Self::SignPdf,
            proto::KeyType::SignChallenge => Self::SignChallenge,
            proto::KeyType::Decrypt => Self::Decrypt,
        }
    }
}

impl From<ProtocolType> for proto::ProtocolType {
    fn from(value: ProtocolType) -> Self {
        match value {
            ProtocolType::Gg18 => Self::Gg18,
            ProtocolType::ElGamal => Self::Elgamal,
            ProtocolType::Frost => Self::Frost,
            ProtocolType::Musig2 => Self::Musig2,
        }
    }
}

impl From<KeyType> for proto::KeyType {
    fn from(value: KeyType) -> Self {
        match value {
            KeyType::SignPdf => proto::KeyType::SignPdf,
            KeyType::SignChallenge => proto::KeyType::SignChallenge,
            KeyType::Decrypt => proto::KeyType::Decrypt,
        }
    }
}
