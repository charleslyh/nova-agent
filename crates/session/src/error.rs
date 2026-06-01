use moray_core::MorayError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("unknown chat session")]
    UnknownSession,

    #[error("{0}")]
    Moray(#[from] MorayError),
}

pub type Result<T> = std::result::Result<T, SessionError>;
