#![forbid(unsafe_code)]
//! Agent skills: disk format, local catalog, SkillHub marketplace, and [`SkillsManager`].

mod catalog;
mod error;
mod hub;
mod load;
mod local;
mod manager;
mod skill;

pub use catalog::{SkillCatalogEntry, SkillDetailEntry};
pub use error::{
    InstallSkillError, SkillHubError, SkillsError, SkillsLoadError, UninstallSkillError,
    UnregisterSkillError, Result,
};
pub use hub::{SearchResult, SkillHub, SkillHubEntry};
pub use load::{
    load_skill_from_dir, load_skill_md, load_skill_toml, load_skills_from_dir,
};
pub use local::LocalSkills;
pub use manager::{SkillsManager, UninstallSkillResult};
pub use skill::Skill;
