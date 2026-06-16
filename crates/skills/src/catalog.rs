//! Serializable catalog rows for skills list/detail HTTP APIs.

use serde::{Deserialize, Serialize};

/// Row for skills list API (`GET /skills`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillCatalogEntry {
    pub id: String,
    /// Directory name under the skills root (matches Hub install slug when installed from hub).
    #[serde(default)]
    pub slug: String,
    pub description: String,
    /// True when loaded from a user-writable directory (safe to uninstall).
    #[serde(default)]
    pub removable: bool,
}

/// Full skill payload for detail API (`GET /skills/{id}`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDetailEntry {
    pub id: String,
    pub description: String,
    pub version: String,
    pub content: String,
    pub location: Option<String>,
    pub always: bool,
}

impl Default for SkillCatalogEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            slug: String::new(),
            description: String::new(),
            removable: false,
        }
    }
}
