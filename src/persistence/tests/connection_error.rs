use thiserror::Error;

#[derive(Error, Debug)]
pub enum TestContextError {
    #[error("diesel error: {0}")]
    DieselError(#[from] diesel::result::Error),
    #[error("diesel connection error: {0}")]
    ConnectionError(#[from] diesel::ConnectionError),
}
