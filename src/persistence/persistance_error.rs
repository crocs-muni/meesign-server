use std::{env::VarError, fmt::Display};

use diesel_async::pooled_connection::deadpool::BuildError;

#[derive(Debug)]
pub enum PersistenceError {
    VarError(VarError),
    PoolBuildError(BuildError),
    InvalidArgumentError(String),
    GeneralError(String),
    ExecutionError(diesel::result::Error),
}

impl Display for PersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "todo error")
    }
}

impl From<VarError> for PersistenceError {
    fn from(value: VarError) -> Self {
        Self::VarError(value)
    }
}

impl From<BuildError> for PersistenceError {
    fn from(value: BuildError) -> Self {
        Self::PoolBuildError(value)
    }
}

impl From<diesel::result::Error> for PersistenceError {
    fn from(value: diesel::result::Error) -> Self {
        Self::ExecutionError(value)
    }
}
