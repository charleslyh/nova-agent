//! Error types for loading skills from disk.

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SkillsLoadError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse error in {path}: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("skill directory has no SKILL.md or SKILL.toml: {0}")]
    NoManifest(PathBuf),
}
