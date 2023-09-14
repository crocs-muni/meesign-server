use std::{error::Error, fmt::Display};

use diesel::QueryResult;

#[derive(Debug)]
pub struct TestContextError {}

impl Error for TestContextError {}

impl Display for TestContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Connection error")
    }
}

impl From<QueryResult<usize>> for TestContextError {
    fn from(_value: QueryResult<usize>) -> Self {
        Self {}
    }
}

impl From<diesel::result::Error> for TestContextError {
    fn from(_value: diesel::result::Error) -> Self {
        Self {}
    }
}

impl From<diesel::ConnectionError> for TestContextError {
    fn from(_value: diesel::ConnectionError) -> Self {
        Self {}
    }
}
