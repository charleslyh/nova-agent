//! [`SkillsSection`] — preamble section for skills injection.

use std::sync::Arc;

use crate::preambles::PreambleSection;

use super::prompt::{skills_authorization_prompt, skills_to_prompt, SkillsPromptMode};
use super::Skill;

/// Renders skills authorization + available skills XML into the system prompt.
pub struct SkillsSection {
    skills: Arc<[Skill]>,
    mode: SkillsPromptMode,
}

impl SkillsSection {
    pub fn new(skills: Vec<Skill>) -> Self {
        Self::with_mode(skills, SkillsPromptMode::default())
    }

    pub fn with_mode(skills: Vec<Skill>, mode: SkillsPromptMode) -> Self {
        Self {
            skills: skills.into(),
            mode,
        }
    }
}

impl PreambleSection for SkillsSection {
    fn render(&self) -> String {
        if self.skills.is_empty() {
            return String::new();
        }

        let names: Vec<String> = self.skills.iter().map(|s| s.name.clone()).collect();
        let mut out = skills_authorization_prompt(&names);
        let skills_block = skills_to_prompt(&self.skills, self.mode);
        if !skills_block.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&skills_block);
        }
        out
    }
}
