//! Render skills into system prompt fragments.

use super::Skill;

/// How much skill content to embed in the system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SkillsPromptMode {
    /// Name, description, and location only (`always` skills still inline instructions).
    #[default]
    Compact,
    /// Full instructions (and tool metadata when present) for every skill.
    Full,
}

/// Authorization block when skills are registered (Moray: tools come from API, not prompt).
pub fn skills_authorization_prompt(skill_names: &[String]) -> String {
    if skill_names.is_empty() {
        return String::new();
    }

    let mut prompt = String::from("## Skills Authorization\n\nAll registered skills (");
    for (i, name) in skill_names.iter().enumerate() {
        if i > 0 {
            prompt.push_str(", ");
        }
        prompt.push_str(name);
    }
    prompt.push_str(") are AUTHORIZED and AVAILABLE for use.\n\n");
    prompt.push_str(
        "**IMPORTANT: Skills are NOT tools.** You cannot call a skill name directly as a tool_call.\n\
         To use a skill:\n\
         1. First call `file_read` on the skill's `location` path to get the SKILL.md instructions\n\
         2. Follow the instructions in SKILL.md (usually involves calling `shell` or other allowed tools)\n\n\
         Do NOT invent tool names based on skill names. Only use tools from the API tool list provided for this turn.\n",
    );
    prompt
}

/// Build the "Available Skills" section with configurable verbosity.
pub fn skills_to_prompt(skills: &[Skill], mode: SkillsPromptMode) -> String {
    use std::fmt::Write;

    if skills.is_empty() {
        return String::new();
    }

    let mut prompt = match mode {
        SkillsPromptMode::Full => String::from(
            "## Available Skills\n\n\
             Skill instructions are preloaded below.\n\
             Follow these instructions directly; do not read skill files at runtime unless the user asks.\n\n\
             <available_skills>\n",
        ),
        SkillsPromptMode::Compact => String::from(
            "## Available Skills\n\n\
             Skill summaries are preloaded below to keep context compact.\n\
             Skill instructions are loaded on demand: read the skill file at `location` when needed. \
             Skills marked `always` include full instructions below even in compact mode.\n\n\
             <available_skills>\n",
        ),
    };

    for skill in skills {
        let _ = writeln!(prompt, "  <skill>");
        write_xml_text_element(&mut prompt, 4, "name", &skill.name);
        write_xml_text_element(&mut prompt, 4, "description", &skill.description);
        write_xml_text_element(&mut prompt, 4, "location", &skill.location_display());

        let inject_full = matches!(mode, SkillsPromptMode::Full) || skill.always;
        if inject_full && !skill.prompts.is_empty() {
            let _ = writeln!(prompt, "    <instructions>");
            for instruction in &skill.prompts {
                write_xml_text_element(&mut prompt, 6, "instruction", instruction);
            }
            let _ = writeln!(prompt, "    </instructions>");
        }

        let _ = writeln!(prompt, "  </skill>");
    }

    prompt.push_str("</available_skills>");
    prompt
}

fn write_xml_text_element(out: &mut String, indent: usize, tag: &str, value: &str) {
    for _ in 0..indent {
        out.push(' ');
    }
    out.push('<');
    out.push_str(tag);
    out.push('>');
    append_xml_escaped(out, value);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

fn append_xml_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_skill(always: bool) -> Skill {
        Skill {
            name: "demo".into(),
            description: "A demo skill".into(),
            version: "1.0.0".into(),
            prompts: vec!["# Do the thing".into()],
            location: Some(PathBuf::from("/tmp/skills/demo/SKILL.md")),
            always,
        }
    }

    #[test]
    fn skills_to_prompt_empty() {
        assert!(skills_to_prompt(&[], SkillsPromptMode::Compact).is_empty());
    }

    #[test]
    fn compact_omits_instructions_unless_always() {
        let out = skills_to_prompt(&[sample_skill(false)], SkillsPromptMode::Compact);
        assert!(out.contains("<available_skills>"));
        assert!(out.contains("<name>demo</name>"));
        assert!(!out.contains("<instructions>"));
    }

    #[test]
    fn compact_inlines_when_always() {
        let out = skills_to_prompt(&[sample_skill(true)], SkillsPromptMode::Compact);
        assert!(out.contains("<instructions>"));
        assert!(out.contains("Do the thing"));
    }

    #[test]
    fn authorization_lists_names() {
        let out = skills_authorization_prompt(&["a".into(), "b".into()]);
        assert!(out.contains("a, b"));
        assert!(out.contains("API tool list"));
    }
}
