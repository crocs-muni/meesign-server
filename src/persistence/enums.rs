use diesel_derive_enum::DbEnum;

#[derive(Copy, Clone, PartialEq, Eq, Debug, DbEnum)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Protocoltype"]
pub enum ProtocolType {
    GG18,
}

#[derive(Debug, DbEnum)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Tasktype"]

pub enum Tasktype {
    Group,
    Sign,
}

#[derive(Debug, DbEnum)]
#[ExistingTypePath = "crate::persistence::schema::sql_types::Taskresulttype"]

pub enum TaskResultType {
    GroupEstablished,
    Signed,
}

#[derive(Debug, DbEnum)]
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
}
