//! Load skills from `SKILL.md` / `SKILL.toml` on disk.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use super::Skill;

#[derive(Debug, Error)]
pub enum SkillsLoadError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("parse error in {path}: {message}")]
    Parse { path: PathBuf, message: String },

    #[error("skill directory has no SKILL.md or SKILL.toml: {0}")]
    NoManifest(PathBuf),
}

/// Load all skills from immediate child directories of `skills_dir`.
pub fn load_skills_from_dir(skills_dir: &Path) -> Result<Vec<Skill>, SkillsLoadError> {
    if !skills_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut skills = Vec::new();
    let entries = fs::read_dir(skills_dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        match load_skill_from_dir(&path) {
            Ok(skill) => skills.push(skill),
            Err(SkillsLoadError::NoManifest(_)) => {}
            Err(e) => tracing::warn!(dir = %path.display(), error = %e, "skipping skill directory"),
        }
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// Load a single skill from an immediate child directory (`SKILL.md` or `SKILL.toml`).
pub fn load_skill_from_dir(dir: &Path) -> Result<Skill, SkillsLoadError> {
    let md = dir.join("SKILL.md");
    let toml = dir.join("SKILL.toml");
    if md.is_file() {
        load_skill_md(&md, dir)
    } else if toml.is_file() {
        load_skill_toml(&toml)
    } else {
        Err(SkillsLoadError::NoManifest(dir.to_path_buf()))
    }
}

/// Load a skill from `SKILL.md` (YAML front matter + markdown body).
pub fn load_skill_md(path: &Path, dir: &Path) -> Result<Skill, SkillsLoadError> {
    let content = fs::read_to_string(path).map_err(|e| SkillsLoadError::Io(e))?;
    let (fm, body) = parse_front_matter(&content);

    let mut name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    if let Some(fm_name) = fm.get("name").filter(|s| !s.is_empty()) {
        name = fm_name.clone();
    }

    let version = fm
        .get("version")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "0.1.0".to_string());

    let always = fm_bool(&fm, "always");
    let prompt_body = if body.trim().is_empty() {
        content.clone()
    } else {
        body.to_string()
    };

    Ok(Skill {
        name,
        description: extract_description(&content, &fm, body),
        version,
        prompts: vec![prompt_body],
        location: Some(path.to_path_buf()),
        always,
    })
}

/// Load a skill from `SKILL.toml`.
pub fn load_skill_toml(path: &Path) -> Result<Skill, SkillsLoadError> {
    let content = fs::read_to_string(path).map_err(|e| SkillsLoadError::Io(e))?;
    let manifest: SkillManifestFile = toml::from_str(&content).map_err(|e| SkillsLoadError::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;

    Ok(Skill {
        name: manifest.skill.name,
        description: manifest.skill.description,
        version: manifest.skill.version.unwrap_or_else(|| "0.1.0".to_string()),
        prompts: manifest.prompts,
        location: Some(path.to_path_buf()),
        always: false,
    })
}

#[derive(Debug, serde::Deserialize)]
struct SkillManifestFile {
    skill: SkillMeta,
    #[serde(default)]
    prompts: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct SkillMeta {
    name: String,
    description: String,
    #[serde(default)]
    version: Option<String>,
}

fn extract_description(_content: &str, fm: &HashMap<String, String>, body: &str) -> String {
    if let Some(desc) = fm.get("description").filter(|s| !s.trim().is_empty()) {
        return desc.trim().to_string();
    }

    body.lines()
        .find(|line| !line.starts_with('#') && !line.trim().is_empty())
        .unwrap_or("No description")
        .trim()
        .to_string()
}

fn fm_bool(map: &HashMap<String, String>, key: &str) -> bool {
    map.get(key)
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "yes" | "1"))
        .unwrap_or(false)
}

fn strip_quotes(s: &str) -> &str {
    let trimmed = s.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    }
}

fn finalize_block_scalar(lines: &[String], literal: bool) -> String {
    if literal {
        lines.join("\n").trim().to_string()
    } else {
        lines
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Parse optional `---` front matter; returns (map, body without front matter).
fn parse_front_matter(content: &str) -> (HashMap<String, String>, &str) {
    let text = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut lines = text.lines().peekable();
    let Some(first) = lines.next() else {
        return (HashMap::new(), content);
    };
    if first.trim() != "---" {
        return (HashMap::new(), content);
    }

    let mut map = HashMap::new();
    let mut end = first.len() + 1;
    let mut block_key: Option<String> = None;
    let mut block_lines: Vec<String> = Vec::new();
    let mut block_is_literal = true;
    let mut block_indent: Option<usize> = None;

    while let Some(line) = lines.next() {
        if line.trim() == "---" {
            if let Some(key) = block_key.take() {
                let value = finalize_block_scalar(&block_lines, block_is_literal);
                if !value.is_empty() {
                    map.insert(key, value);
                }
                block_lines.clear();
            }
            let body_start = end + line.len() + 1;
            let body = if body_start <= text.len() {
                text[body_start..].trim_start_matches(['\n', '\r'])
            } else {
                ""
            };
            return (map, body);
        }

        if block_key.is_some() {
            let line_indent = line.len() - line.trim_start().len();
            if block_indent.is_none() && !line.trim().is_empty() {
                block_indent = Some(line_indent);
            }
            if let Some(bi) = block_indent {
                if line_indent >= bi || line.trim().is_empty() {
                    let content_line = if line.len() > bi && !line.trim().is_empty() {
                        &line[bi..]
                    } else if line.trim().is_empty() {
                        ""
                    } else {
                        line.trim()
                    };
                    block_lines.push(content_line.to_string());
                    end += line.len() + 1;
                    continue;
                }
            } else if line.trim().is_empty() {
                block_lines.push(String::new());
                end += line.len() + 1;
                continue;
            }

            let key = block_key.take().unwrap();
            let value = finalize_block_scalar(&block_lines, block_is_literal);
            if !value.is_empty() {
                map.insert(key, value);
            }
            block_lines.clear();
            block_indent = None;
        }

        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_lowercase();
            let value_trimmed = value.trim();
            if key.is_empty() {
                end += line.len() + 1;
                continue;
            }
            if value_trimmed == "|" || value_trimmed == "|-" || value_trimmed == "|+" {
                block_key = Some(key);
                block_is_literal = true;
                block_lines.clear();
                block_indent = None;
            } else if value_trimmed == ">" || value_trimmed == ">-" || value_trimmed == ">+" {
                block_key = Some(key);
                block_is_literal = false;
                block_lines.clear();
                block_indent = None;
            } else {
                let value = strip_quotes(value).to_string();
                if !value.is_empty() {
                    map.insert(key, value);
                }
            }
        }
        end += line.len() + 1;
    }

    (map, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_skill_md_parses_front_matter_and_body() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("demo-skill");
        fs::create_dir_all(&skill_dir).unwrap();
        let path = skill_dir.join("SKILL.md");
        fs::write(
            &path,
            r#"---
name: demo
description: |
  Line one
  Line two
always: true
---
# Title
Body here.
"#,
        )
        .unwrap();

        let skill = load_skill_md(&path, &skill_dir).unwrap();
        assert_eq!(skill.name, "demo");
        assert!(skill.description.contains("Line one"));
        assert!(skill.always);
        assert!(skill.prompts[0].contains("# Title"));
        assert_eq!(skill.location.as_ref().unwrap(), &path);
    }

    #[test]
    fn load_skills_from_dir_skips_empty_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let skills_root = dir.path().join("skills");
        fs::create_dir_all(&skills_root.join("good")).unwrap();
        fs::write(
            skills_root.join("good/SKILL.md"),
            "---\nname: good\ndescription: ok\n---\n",
        )
        .unwrap();
        fs::create_dir_all(&skills_root.join("bad")).unwrap();

        let skills = load_skills_from_dir(&skills_root).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "good");
    }
}
