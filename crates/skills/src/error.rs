//! Unified error types for the `moray-skills` crate.

use std::path::PathBuf;

use thiserror::Error;

/// Crate-wide [`Result`] alias; use `std::result::Result<T, E>` when a narrower error is required.
pub type Result<T> = std::result::Result<T, SkillsError>;

#[derive(Debug, Error)]
pub enum SkillsError {
    #[error(transparent)]
    Load(#[from] SkillsLoadError),

    #[error("{0}")]
    Catalog(String),

    #[error(transparent)]
    Unregister(#[from] UnregisterSkillError),

    #[error(transparent)]
    Install(#[from] InstallSkillError),

    #[error(transparent)]
    Uninstall(#[from] UninstallSkillError),

    #[error(transparent)]
    Hub(#[from] SkillHubError),
}

#[derive(Debug, Error)]
pub enum SkillsLoadError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse error in {path}: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("skill directory has no SKILL.md or SKILL.toml: {0}")]
    NoManifest(PathBuf),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{message}")]
pub(crate) struct CatalogError {
    pub message: String,
}

impl CatalogError {
    #[inline]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<CatalogError> for SkillsError {
    fn from(err: CatalogError) -> Self {
        Self::Catalog(err.message)
    }
}

impl From<CatalogError> for UnregisterSkillError {
    fn from(err: CatalogError) -> Self {
        Self::Catalog(err.message)
    }
}

#[derive(Debug, Error)]
pub enum UnregisterSkillError {
    #[error("invalid skill id")]
    InvalidId,

    #[error("unknown skill")]
    NotFound,

    #[error("skill is not removable")]
    NotRemovable,

    #[error(transparent)]
    Load(#[from] SkillsLoadError),

    #[error("{0}")]
    Catalog(String),
}

#[derive(Debug, Error)]
pub enum InstallSkillError {
    #[error(transparent)]
    Hub(#[from] SkillHubError),

    #[error("downloaded but catalog registration failed: {0}")]
    RegisterFailed(String),
}

#[derive(Debug, Error)]
pub enum UninstallSkillError {
    #[error(transparent)]
    Unregister(#[from] UnregisterSkillError),

    #[error("failed to remove skill files: {0}")]
    RemoveFiles(String),
}

#[derive(Debug, Error)]
pub enum SkillHubError {
    #[error("invalid slug: {0}")]
    InvalidSlug(String),

    #[error("skill slug must not be empty")]
    EmptySlug,

    #[error("skill '{slug}' is already downloaded at {path} (use force to overwrite)")]
    AlreadyDownloaded { slug: String, path: PathBuf },

    #[error("no download URL candidates for skill '{0}'")]
    NoDownloadUrls(String),

    #[error("all download attempts failed for skill '{slug}': {detail}")]
    DownloadFailed { slug: String, detail: String },

    #[error("SHA256 mismatch for '{slug}': expected {expected}, got {actual}")]
    Sha256Mismatch {
        slug: String,
        expected: String,
        actual: String,
    },

    #[error("search URL is empty")]
    EmptySearchUrl,

    #[error("skills index URL is empty")]
    EmptyIndexUrl,

    #[error("HTTP {status} from {url}")]
    Http { status: u16, url: String },

    #[error("failed to parse JSON: {0}")]
    Json(String),

    #[error("response is not valid UTF-8")]
    Utf8,

    #[error("downloaded content is not a valid zip archive")]
    InvalidZip,

    #[error("unsafe zip entry: {0}")]
    UnsafeZipEntry(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
