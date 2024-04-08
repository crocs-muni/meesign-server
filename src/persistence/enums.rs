use crate::proto;
use diesel_derive_enum::DbEnum;

#[derive(Copy, Clone, PartialEq, Eq, Debug, DbEnum)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Protocoltype"]
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

pub enum TaskType {
    Group,
    SignPdf,
    SignChallenge,
    Decrypt,
}

#[derive(Debug, DbEnum)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Taskresulttype"]

pub enum TaskResultType {
    GroupEstablished,
    Signed,
    SignedPdf,
    Decrypted,
}

#[derive(Debug, DbEnum, Clone, PartialEq, Eq)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Taskstate"]

pub enum TaskState {
    Created,
    Running,
    Finished,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, DbEnum)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Keytype"]

pub enum KeyType {
    SignPDF,
    SignChallenge,
    Decrypt,
}

impl From<proto::KeyType> for KeyType {
    fn from(value: proto::KeyType) -> Self {
        match value {
            proto::KeyType::SignPdf => Self::SignPDF,
            proto::KeyType::SignChallenge => Self::SignChallenge,
            proto::KeyType::Decrypt => Self::Decrypt,
        }
    }
}
