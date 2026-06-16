//! Agent skills: load from disk and render into system prompt sections.
//!
//! For local catalog, SkillHub client, and install/uninstall orchestration, see `moray-skills`
//! (`LocalSkills`, `SkillHub`, `SkillsManager`).

mod prompt;
mod section;

pub use moray_skills::{
    load_skill_from_dir, load_skill_md, load_skill_toml, load_skills_from_dir, Skill,
    SkillsLoadError,
};
pub use prompt::{skills_authorization_prompt, skills_to_prompt, SkillsPromptMode};
pub use section::SkillsSection;
