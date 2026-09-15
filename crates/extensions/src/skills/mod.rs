//! Agent skills: load from disk and render into system prompt sections.

mod error;
mod load;
mod prompt;
mod section;
mod skill;

pub use error::SkillsLoadError;
pub use load::{load_skill_from_dir, load_skill_md, load_skill_toml, load_skills_from_dir};
pub use prompt::{skills_authorization_prompt, skills_to_prompt, SkillsPromptMode};
pub use section::SkillsSection;
pub use skill::Skill;
