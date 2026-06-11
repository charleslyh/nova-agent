#![forbid(unsafe_code)]
//! Agent skill types and on-disk loading (`SKILL.md` / `SKILL.toml`).

mod load;
mod skill;

pub use load::{
    load_skill_from_dir, load_skill_md, load_skill_toml, load_skills_from_dir, SkillsLoadError,
};
pub use skill::Skill;
