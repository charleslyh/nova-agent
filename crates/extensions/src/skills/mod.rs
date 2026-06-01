//! Agent skills: load from disk and render into system prompt sections.
//!
//! For the external SkillHub marketplace (search/download), see the `moray-skillhub` crate.

mod load;
mod prompt;
mod section;

pub use load::{
    load_skill_from_dir, load_skill_md, load_skill_toml, load_skills_from_dir, SkillsLoadError,
};
pub use prompt::{skills_authorization_prompt, skills_to_prompt, SkillsPromptMode};
pub use section::SkillsSection;

use std::path::PathBuf;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A loaded agent skill (metadata + optional inlined instructions).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
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
