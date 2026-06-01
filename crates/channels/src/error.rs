//! Channel catalog, connector, and live-events error types.

use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, ChannelCatalogError>;

#[derive(Debug, Error)]
pub enum ChannelError {
    #[error("{0}")]
    Op(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("websocket: {0}")]
    Tungstenite(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("listener join: {0}")]
    ListenerJoin(#[from] tokio::task::JoinError),

    #[error("max reconnect attempts ({limit}) exceeded")]
    MaxReconnect { limit: u32 },
}

impl ChannelError {
    pub fn op(msg: impl Into<String>) -> Self {
        Self::Op(msg.into())
    }
}

#[derive(Debug, Error)]
pub enum ChannelCatalogError {
    #[error(transparent)]
    InvalidContent(#[from] InvalidContent),

    #[error(transparent)]
    MissingReference(#[from] MissingReference),

    #[error(transparent)]
    FileIo(#[from] FileIoError),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct InvalidContent {
    pub message: String,
}

impl InvalidContent {
    #[inline]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub struct MissingReference {
    pub message: String,
}

impl MissingReference {
    #[inline]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Error)]
#[error("Failed to {op} `{path}`: {source}")]
pub struct FileIoError {
    pub op: &'static str,
    pub path: PathBuf,
    #[source]
    pub source: std::io::Error,
}

impl FileIoError {
    #[inline]
    pub fn new(op: &'static str, path: PathBuf, source: std::io::Error) -> Self {
        Self { op, path, source }
    }
}

/// Returns `raw` trimmed when non-empty; otherwise [`InvalidContent`] naming `field`.
pub fn require_nonempty_trimmed(
    raw: &str,
    field: &'static str,
) -> std::result::Result<String, InvalidContent> {
    require_nonempty_field(raw, field)?;
    Ok(raw.trim().to_string())
}

/// Validates that `raw` is non-empty after trim (config / persisted fields).
pub fn require_nonempty_field(
    raw: &str,
    field: &'static str,
) -> std::result::Result<(), InvalidContent> {
    if raw.trim().is_empty() {
        return Err(InvalidContent::new(format!(
            "field `{field}` is missing or empty"
        )));
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SessionLiveEventsError {
    #[error("session not found")]
    NotFound,
    #[error("{0}")]
    Message(String),
}
