use std::{env::VarError, num::TryFromIntError};

use diesel_async::pooled_connection::deadpool::BuildError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PersistenceError {
    #[error("VarError: {0}")]
    VarError(#[from] VarError),
    #[error("PoolBuildError: {0}")]
    PoolBuildError(#[from] BuildError),
    #[error("InvalidArgumentError: {0}")]
    InvalidArgumentError(String),
    #[error("GeneralError: {0}")]
    GeneralError(String),
    #[error("ExecutionError: {0}")]
    ExecutionError(#[from] diesel::result::Error),
    #[error("TryFromIntError: {0}")]
    TryFromIntError(#[from] TryFromIntError),
    #[cfg(test)]
    #[error("diesel connection error: {0}")]
    ConnectionError(#[from] diesel::ConnectionError),
}

impl From<PersistenceError> for tonic::Status {
    fn from(_value: PersistenceError) -> Self {
        Self::internal("Internal error occurred")
    }
}
