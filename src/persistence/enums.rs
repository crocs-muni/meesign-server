use crate::proto;
use diesel_derive_enum::DbEnum;

#[derive(Copy, Clone, PartialEq, Eq, Debug, DbEnum)]
#[cfg_attr(test, derive(PartialOrd, Ord))]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Protocoltype"]
#[DbValueStyle = "PascalCase"]
pub enum ProtocolType {
    Gg18,
    ElGamal,
    Frost,
}

impl From<proto::ProtocolType> for ProtocolType {
    fn from(value: proto::ProtocolType) -> Self {
        match value {
            proto::ProtocolType::Gg18 => Self::Gg18,
            proto::ProtocolType::Elgamal => Self::ElGamal,
            proto::ProtocolType::Frost => Self::Frost,
        }
    }
}

#[derive(Debug, DbEnum, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Tasktype"]
#[DbValueStyle = "PascalCase"]
pub enum TaskType {
    Group,
    SignPdf,
    SignChallenge,
    Decrypt,
}

#[derive(Debug, DbEnum)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Taskresulttype"]
#[DbValueStyle = "PascalCase"]
pub enum TaskResultType {
    GroupEstablished,
    Signed,
    SignedPdf,
    Decrypted,
}

#[derive(Debug, DbEnum, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Taskstate"]
#[DbValueStyle = "PascalCase"]
pub enum TaskState {
    Created,
    Running,
    Finished,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, DbEnum)]
#[cfg_attr(test, derive(PartialOrd, Ord))]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Keytype"]
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
