//! [`SkillsSection`] — preamble section for skills injection.

use std::sync::Mutex;

use crate::preambles::PreambleSection;

use super::prompt::{skills_authorization_prompt, skills_to_prompt, SkillsPromptMode};
use super::Skill;

/// Renders skills authorization + available skills XML into the system prompt.
pub struct SkillsSection {
    resolve: Mutex<Box<dyn FnMut() -> Vec<Skill> + Send>>,
    mode: SkillsPromptMode,
}

impl SkillsSection {
    /// Skills are resolved on each [`PreambleSection::render`].
    pub fn new<F>(resolve: F) -> Self
    where
        F: FnMut() -> Vec<Skill> + Send + 'static,
    {
        Self {
            resolve: Mutex::new(Box::new(resolve)),
            mode: SkillsPromptMode::default(),
        }
    }

    pub fn mode(mut self, mode: SkillsPromptMode) -> Self {
        self.mode = mode;
        self
    }

    fn skills(&self) -> Vec<Skill> {
        match self.resolve.lock() {
            Ok(mut f) => f(),
            Err(_) => Vec::new(),
        }
    }

    fn render_skills(skills: &[Skill], mode: SkillsPromptMode) -> String {
        if skills.is_empty() {
            return String::new();
        }

        let names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
        let mut out = skills_authorization_prompt(&names);
        let skills_block = skills_to_prompt(skills, mode);
        if !skills_block.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&skills_block);
        }
        out
    }
}

impl PreambleSection for SkillsSection {
    fn render(&self) -> String {
        Self::render_skills(&self.skills(), self.mode)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    fn sample_skill(name: &str) -> Skill {
        Skill {
            name: name.into(),
            description: format!("{name} desc"),
            version: "1".into(),
            prompts: vec![],
            location: None,
            always: false,
        }
    }

    #[test]
    fn resolve_invoked_each_render() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_in_fn = calls.clone();
        let section = SkillsSection::new(move || {
            calls_in_fn.fetch_add(1, Ordering::SeqCst);
            vec![sample_skill("alpha")]
        });
        section.render();
        section.render();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn resolve_reads_catalog_at_render_time() {
        let catalog = Arc::new(Mutex::new(vec![sample_skill("alpha")]));
        let catalog_in_fn = catalog.clone();
        let section = SkillsSection::new(move || catalog_in_fn.lock().unwrap().clone());
        assert!(section.render().contains("alpha"));
        *catalog.lock().unwrap() = vec![sample_skill("beta")];
        let updated = section.render();
        assert!(updated.contains("beta"));
        assert!(!updated.contains("alpha"));
    }
}
