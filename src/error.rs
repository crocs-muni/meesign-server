use log::error;
use thiserror::Error;

use crate::persistence::PersistenceError;

#[derive(Error, Debug)]
pub(crate) enum Error {
    #[error("PersistenceError: {0}")]
    PersistenceError(#[from] PersistenceError),
    #[error("GeneralProtocolError: {0}")]
    GeneralProtocolError(String),
}

impl From<String> for Error {
    fn from(value: String) -> Self {
        Self::GeneralProtocolError(value)
    }
}

impl From<Error> for tonic::Status {
    fn from(value: Error) -> Self {
        match value {
            Error::PersistenceError(_) => Self::internal("Internal error occurred"),
            Error::GeneralProtocolError(error) => Self::failed_precondition(error),
        }
    }
}
