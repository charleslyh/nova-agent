//! Loaded agent skill metadata and instructions.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A loaded agent skill (metadata + optional inlined instructions).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub prompts: Vec<String>,
    #[serde(skip)]
    pub location: Option<PathBuf>,
    /// When true, include full instructions even in compact prompt mode.
    #[serde(default)]
    pub always: bool,
}

impl Skill {
    pub fn location_display(&self) -> String {
        self.location
            .as_ref()
            .map(|p| p.display().to_string().replace('\\', "/"))
            .unwrap_or_default()
    }
}
