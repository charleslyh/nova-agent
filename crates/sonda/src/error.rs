use std::path::PathBuf;

use crate::SondaToolCatalogError;
use moray_core::MorayError;
use moray_skills::SkillsError;
use thiserror::Error;

pub use moray_channels::ChannelCatalogError;
pub use super::session_catalog::SessionCatalogError;
pub use super::settings_store::SondaSettingsStoreError;

pub use moray_session::SessionError as SondaSessionError;

/// Sonda 桌面服务端统一错误类型：通用错误 + 各子模块错误聚合。
#[derive(Debug, Error)]
pub enum SondaError {
    #[error(transparent)]
    InvalidArguments(#[from] InvalidArguments),

    #[error(transparent)]
    InvalidContent(#[from] InvalidContent),

    #[error(transparent)]
    MissingReference(#[from] MissingReference),

    #[error(transparent)]
    FileIo(#[from] FileIoError),

    #[error(transparent)]
    BadEnvironmentVariable(#[from] BadEnvironmentVariable),

    #[error(transparent)]
    SettingsStore(#[from] SondaSettingsStoreError),

    #[error(transparent)]
    SessionCatalog(#[from] SessionCatalogError),

    #[error(transparent)]
    ChannelCatalog(#[from] ChannelCatalogError),

    #[error(transparent)]
    ToolCatalog(#[from] SondaToolCatalogError),

    #[error(transparent)]
    Skills(#[from] SkillsError),
}

impl From<crate::transcripts::SondaSessionTranscriptsError> for SondaError {
    #[inline]
    fn from(err: crate::transcripts::SondaSessionTranscriptsError) -> Self {
        InvalidContent::new(err.to_string()).into()
    }
}

impl From<SondaError> for MorayError {
    #[inline]
    fn from(err: SondaError) -> Self {
        MorayError::Message(err.to_string())
    }
}

/// Crate 内统一 [`Result`]，错误固定为 [`SondaError`]。
///
/// 子模块应定义各自的 `Result` 别名；跨模块编排（如 [`super::sonda`]）可直接使用本别名。
/// 需指标准库时请写 `std::result::Result<T, E>`。
///
/// 函数签名应尽量收窄错误语义（优先级从高到低）：
/// 1. 具体错误（如 `InvalidArguments`、`InvalidContent`）
/// 2. 子模块 `Result`（如 `settings_store::Result`）
/// 3. 本 crate 的 [`Result`]（[`SondaError`] 聚合）
///
/// 仅在边界（HTTP 路由、bootstrap）将子模块错误用 `?` / `From` 升格为 [`SondaError`]。
pub type Result<T> = std::result::Result<T, SondaError>;

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

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("invalid argument `{name}`, {message}")]
pub struct InvalidArguments {
    pub name: &'static str,
    pub message: String,
}

impl InvalidArguments {
    #[inline]
    pub fn new(name: &'static str, message: impl Into<String>) -> Self {
        Self {
            name,
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

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BadEnvironmentVariable {
    #[error("environment variable `{name}` is not set")]
    NotSet { name: String },

    #[error("environment variable `{name}` is empty")]
    Empty { name: String },
}

impl BadEnvironmentVariable {
    #[inline]
    pub fn not_set(name: impl Into<String>) -> Self {
        Self::NotSet { name: name.into() }
    }

    #[inline]
    pub fn empty(name: impl Into<String>) -> Self {
        Self::Empty { name: name.into() }
    }
}

pub fn require_argument_nonempty<'a>(raw: &'a str, name: &'static str) -> Result<&'a str> {
    let s = raw.trim();
    if s.is_empty() {
        return Err(InvalidArguments::new(name, "is empty").into());
    }
    Ok(s)
}

/// Returns `raw` trimmed when non-empty; otherwise [`InvalidContent`] naming `field`.
pub fn require_nonempty_trimmed(
    raw: &str,
    field: &'static str,
) -> std::result::Result<String, InvalidContent> {
    require_nonempty_field(raw, field)?;
    // `trim()` is applied again; `require_nonempty_field` only checks non-emptiness.
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
