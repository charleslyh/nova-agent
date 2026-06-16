//! In-process catalog of **local** skills for HTTP APIs and preamble assembly.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::catalog::{SkillCatalogEntry, SkillDetailEntry};
use crate::error::{CatalogError, Result, SkillsError, UnregisterSkillError};
use crate::load::{load_skill_from_dir, load_skills_from_dir};
use crate::Skill;

/// Disk location removed from the catalog (filesystem delete is the caller's job).
pub(crate) struct UnregisterResult {
    pub slug: String,
    pub user_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillDirKind {
    User,
    Bundled,
}

#[derive(Debug, Clone)]
struct SkillDirSource {
    path: PathBuf,
    kind: SkillDirKind,
}

impl SkillDirSource {
    fn new(path: impl Into<PathBuf>, kind: SkillDirKind) -> Self {
        Self {
            path: path.into(),
            kind,
        }
    }
}

#[derive(Clone)]
struct RegisteredSkill {
    entry: SkillCatalogEntry,
    skill: Skill,
}

/// In-memory skill catalog state (wrapped by [`LocalSkills`] for thread-safe access).
struct LocalSkillsInner {
    sources: Vec<SkillDirSource>,
    order: Vec<String>,
    skills: HashMap<String, RegisteredSkill>,
}

impl LocalSkillsInner {
    /// Loads skills from `sources` in order; earlier sources win when ids collide.
    fn load(sources: impl IntoIterator<Item = SkillDirSource>) -> Result<Self> {
        let sources: Vec<_> = sources.into_iter().collect();
        let mut center = Self {
            sources,
            order: Vec::new(),
            skills: HashMap::new(),
        };
        center.reload()?;
        Ok(center)
    }

    fn dir(&self, kind: SkillDirKind) -> Option<&Path> {
        self.sources
            .iter()
            .find(|s| s.kind == kind)
            .map(|s| s.path.as_path())
    }

    fn skills_dir_user(&self) -> Option<&Path> {
        self.dir(SkillDirKind::User)
    }

    fn skills_dir_bundled(&self) -> Option<&Path> {
        self.dir(SkillDirKind::Bundled)
    }

    /// Full rescan of [`Self::sources`]. Not used by install/uninstall — only startup and explicit refresh.
    fn reload(&mut self) -> Result<()> {
        self.order.clear();
        self.skills.clear();

        let sources = self.sources.clone();
        for source in &sources {
            let loaded = load_skills_from_dir(&source.path)?;

            for skill in loaded {
                if self.skills.contains_key(&skill.name) {
                    continue;
                }
                self.insert_skill(skill, source.kind);
            }
        }
        Ok(())
    }

    /// Add or update a user skill in the catalog from an on-disk directory (e.g. after SkillHub download).
    fn register_from_dir(&mut self, skill_dir: &Path) -> Result<()> {
        let skill = load_skill_from_dir(skill_dir).map_err(|e| {
            CatalogError::new(format!(
                "load skill from {}: {e}",
                skill_dir.display()
            ))
        })?;
        let id = skill.name.clone();
        if self.skills.contains_key(&id) {
            let entry = catalog_entry(&skill, SkillDirKind::User);
            if let Some(r) = self.skills.get_mut(&id) {
                r.entry = entry;
                r.skill = skill;
            }
        } else {
            self.insert_skill(skill, SkillDirKind::User);
        }
        Ok(())
    }

    /// Remove a user skill from the catalog and restore a bundled default when present.
    fn unregister(
        &mut self,
        id: &str,
    ) -> std::result::Result<UnregisterResult, UnregisterSkillError> {
        if !self.is_known(id) {
            return Err(UnregisterSkillError::NotFound);
        }
        if !self.is_removable(id) {
            return Err(UnregisterSkillError::NotRemovable);
        }

        let user_dir = self.user_skill_dir(id).map_err(map_catalog_err)?;
        let slug = user_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(id)
            .to_string();
        self.remove_entry(id);
        self.restore_bundled_if_missing(id).map_err(map_catalog_err)?;
        Ok(UnregisterResult { slug, user_dir })
    }

    fn user_skill_dir(&self, id: &str) -> Result<PathBuf> {
        let detail = self
            .detail(id)
            .ok_or_else(|| CatalogError::new(format!("unknown skill id `{id}`")))?;
        let location = detail
            .location
            .as_deref()
            .ok_or_else(|| CatalogError::new(format!("skill `{id}` has no location")))?;
        let skill_dir = Path::new(location)
            .parent()
            .ok_or_else(|| CatalogError::new(format!("invalid skill location for `{id}`")))?;

        if skill_dir
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(CatalogError::new(format!("invalid skill location for `{id}`")).into());
        }

        let Some(user_root) = self.skills_dir_user() else {
            return Err(CatalogError::new("user skills directory not configured").into());
        };
        let user_root = std::fs::canonicalize(user_root).map_err(|e| {
            CatalogError::new(format!(
                "canonicalize user skills dir {}: {e}",
                user_root.display()
            ))
        })?;
        let canonical_skill = std::fs::canonicalize(skill_dir).map_err(|e| {
            CatalogError::new(format!(
                "canonicalize skill dir {}: {e}",
                skill_dir.display()
            ))
        })?;
        if !canonical_skill.starts_with(&user_root) {
            return Err(CatalogError::new(format!("skill `{id}` is not removable")).into());
        }

        Ok(canonical_skill)
    }

    #[cfg(test)]
    /// Registers a skill for unit tests. `id` must match [`Skill::name`] and be unique.
    pub fn register(&mut self, skill: Skill) -> Result<()> {
        let id = skill.name.clone();
        if self.skills.contains_key(&id) {
            return Err(CatalogError::new(format!("duplicate skill id `{id}`")).into());
        }
        self.insert_skill(skill, SkillDirKind::Bundled);
        Ok(())
    }

    fn remove_entry(&mut self, id: &str) {
        if self.skills.remove(id).is_some() {
            self.order.retain(|existing| existing != id);
        }
    }

    fn restore_bundled_if_missing(&mut self, id: &str) -> Result<()> {
        if self.is_known(id) {
            return Ok(());
        }
        let Some(bundled) = self.skills_dir_bundled() else {
            return Ok(());
        };
        let loaded = load_skills_from_dir(bundled)?;
        if let Some(skill) = loaded.into_iter().find(|s| s.name == id) {
            self.insert_skill(skill, SkillDirKind::Bundled);
        }
        Ok(())
    }

    fn insert_skill(&mut self, skill: Skill, kind: SkillDirKind) {
        let id = skill.name.clone();
        let entry = catalog_entry(&skill, kind);
        self.order.push(id.clone());
        self.skills.insert(
            id,
            RegisteredSkill {
                entry,
                skill,
            },
        );
    }

    fn is_removable(&self, id: &str) -> bool {
        self.skills
            .get(id)
            .is_some_and(|r| r.entry.removable)
    }

    fn catalog(&self) -> Vec<SkillCatalogEntry> {
        self.order
            .iter()
            .filter_map(|id| self.skills.get(id).map(|r| r.entry.clone()))
            .collect()
    }

    fn detail(&self, id: &str) -> Option<SkillDetailEntry> {
        self.skills.get(id).map(|r| {
            let s = &r.skill;
            let location = s.location_display();
            SkillDetailEntry {
                id: s.name.clone(),
                description: s.description.clone(),
                version: s.version.clone(),
                content: s.prompts.join("\n\n"),
                location: if location.is_empty() {
                    None
                } else {
                    Some(location)
                },
                always: s.always,
            }
        })
    }

    fn is_known(&self, id: &str) -> bool {
        self.skills.contains_key(id)
    }

    fn collect(&self, ids: Option<&[String]>) -> Vec<Skill> {
        match ids {
            None => self
                .order
                .iter()
                .filter_map(|id| self.skills.get(id).map(|r| r.skill.clone()))
                .collect(),
            Some(ids) => {
                let wanted: HashSet<&str> = ids.iter().map(String::as_str).collect();
                self.order
                    .iter()
                    .filter(|id| wanted.contains(id.as_str()))
                    .filter_map(|id| self.skills.get(id).map(|r| r.skill.clone()))
                    .collect()
            }
        }
    }
}

fn map_catalog_err(err: SkillsError) -> UnregisterSkillError {
    match err {
        SkillsError::Load(e) => UnregisterSkillError::Load(e),
        SkillsError::Catalog(msg) => UnregisterSkillError::Catalog(msg),
        other => UnregisterSkillError::Catalog(other.to_string()),
    }
}

fn skill_dir_slug(skill: &Skill) -> String {
    skill
        .location
        .as_ref()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or(skill.name.as_str())
        .to_string()
}

fn catalog_entry(skill: &Skill, kind: SkillDirKind) -> SkillCatalogEntry {
    SkillCatalogEntry {
        id: skill.name.clone(),
        slug: skill_dir_slug(skill),
        description: skill.description.clone(),
        removable: kind == SkillDirKind::User,
    }
}

/// Thread-safe local skill catalog (HTTP APIs and preamble assembly).
#[derive(Clone)]
pub struct LocalSkills {
    inner: Arc<RwLock<LocalSkillsInner>>,
}

impl LocalSkills {
    fn new(inner: LocalSkillsInner) -> Self {
        Self {
            inner: Arc::new(RwLock::new(inner)),
        }
    }

    pub(crate) fn load(user_dir: impl AsRef<Path>, bundled_dir: impl AsRef<Path>) -> Result<Self> {
        Self::load_from_sources([
            SkillDirSource::new(user_dir.as_ref().to_path_buf(), SkillDirKind::User),
            SkillDirSource::new(bundled_dir.as_ref().to_path_buf(), SkillDirKind::Bundled),
        ])
    }

    fn load_from_sources(sources: impl IntoIterator<Item = SkillDirSource>) -> Result<Self> {
        Ok(Self::new(LocalSkillsInner::load(sources)?))
    }

    pub fn catalog(&self) -> Vec<SkillCatalogEntry> {
        self.with_read(|c| c.catalog())
    }

    pub fn detail(&self, id: &str) -> Option<SkillDetailEntry> {
        self.with_read(|c| c.detail(id))
    }

    /// All registered skills in registration order (for preamble / agent context).
    pub fn all(&self) -> Vec<Skill> {
        self.with_read(|c| c.collect(None))
    }

    /// Subset of registered skills by id, in registration order (unknown ids skipped).
    pub fn by_ids(&self, ids: &[String]) -> Vec<Skill> {
        self.with_read(|c| c.collect(Some(ids)))
    }

    pub(crate) fn register_dir(&self, skill_dir: &Path) -> Result<()> {
        self.with_write(|c| c.register_from_dir(skill_dir))
    }

    pub(crate) fn unregister(
        &self,
        skill_id: &str,
    ) -> std::result::Result<UnregisterResult, UnregisterSkillError> {
        if skill_id.contains("..") || skill_id.contains('/') || skill_id.contains('\\') {
            return Err(UnregisterSkillError::InvalidId);
        }

        self.with_write(|c| c.unregister(skill_id))
    }

    fn with_read<R>(&self, f: impl FnOnce(&LocalSkillsInner) -> R) -> R {
        f(&self
            .inner
            .read()
            .expect("local_skills lock poisoned"))
    }

    fn with_write<R>(&self, f: impl FnOnce(&mut LocalSkillsInner) -> R) -> R {
        f(&mut self
            .inner
            .write()
            .expect("local_skills lock poisoned"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_skill(name: &str) -> Skill {
        Skill {
            name: name.into(),
            description: "desc".into(),
            version: "0.1.0".into(),
            prompts: vec![],
            location: Some(PathBuf::from("/tmp/x/SKILL.md")),
            always: false,
        }
    }

    fn load_fixture(user: &Path, bundled: &Path) -> LocalSkillsInner {
        LocalSkillsInner::load([
            SkillDirSource::new(user, SkillDirKind::User),
            SkillDirSource::new(bundled, SkillDirKind::Bundled),
        ])
        .unwrap()
    }

    #[test]
    fn register_rejects_duplicate_id() {
        let mut c = LocalSkillsInner::load(Vec::<SkillDirSource>::new()).unwrap();
        c.register(sample_skill("a")).unwrap();
        assert!(c.register(sample_skill("a")).is_err());
    }

    #[test]
    fn all_preserves_registration_order() {
        let mut c = LocalSkillsInner::load(Vec::<SkillDirSource>::new()).unwrap();
        c.register(sample_skill("b")).unwrap();
        c.register(sample_skill("a")).unwrap();
        let names: Vec<_> = c
            .collect(None)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["b", "a"]);
    }

    #[test]
    fn catalog_marks_removable_from_source_kind() {
        let base = tempfile::tempdir().unwrap();
        let user = base.path().join("user");
        let bundled = base.path().join("bundled");
        std::fs::create_dir_all(user.join("user-skill")).unwrap();
        std::fs::write(
            user.join("user-skill/SKILL.md"),
            "---\nname: user-skill\ndescription: u\n---\n",
        )
        .unwrap();
        std::fs::create_dir_all(bundled.join("bundled-skill")).unwrap();
        std::fs::write(
            bundled.join("bundled-skill/SKILL.md"),
            "---\nname: bundled-skill\ndescription: b\n---\n",
        )
        .unwrap();

        let c = load_fixture(&user, &bundled);
        let catalog = c.catalog();
        let user_entry = catalog.iter().find(|e| e.id == "user-skill").unwrap();
        let bundled_entry = catalog.iter().find(|e| e.id == "bundled-skill").unwrap();
        assert!(user_entry.removable);
        assert!(!bundled_entry.removable);
    }

    #[test]
    fn load_first_source_wins_for_duplicate_ids() {
        let base = tempfile::tempdir().unwrap();
        let user = base.path().join("user");
        let bundled = base.path().join("bundled");
        std::fs::create_dir_all(user.join("skill-a")).unwrap();
        std::fs::write(
            user.join("skill-a/SKILL.md"),
            "---\nname: a\ndescription: user override\n---\n",
        )
        .unwrap();
        std::fs::create_dir_all(bundled.join("skill-a")).unwrap();
        std::fs::write(
            bundled.join("skill-a/SKILL.md"),
            "---\nname: a\ndescription: bundled default\n---\n",
        )
        .unwrap();
        std::fs::create_dir_all(bundled.join("skill-b")).unwrap();
        std::fs::write(
            bundled.join("skill-b/SKILL.md"),
            "---\nname: b\ndescription: bundled only\n---\n",
        )
        .unwrap();

        let c = load_fixture(&user, &bundled);
        let catalog = c.catalog();
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog[0].id, "a");
        assert_eq!(catalog[0].description, "user override");
        assert_eq!(catalog[1].id, "b");
    }

    #[test]
    fn register_from_dir_replaces_existing_without_reordering() {
        let mut c = LocalSkillsInner::load(Vec::<SkillDirSource>::new()).unwrap();
        c.register(sample_skill("a")).unwrap();
        c.register(sample_skill("b")).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("a");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: a\ndescription: updated\n---\n",
        )
        .unwrap();

        c.register_from_dir(&skill_dir).unwrap();
        let names: Vec<_> = c
            .collect(None)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["a", "b"]);
        assert_eq!(c.detail("a").unwrap().description, "updated");
        assert!(c.catalog().iter().find(|e| e.id == "a").unwrap().removable);
    }

    #[test]
    fn unregister_restores_bundled_default() {
        let base = tempfile::tempdir().unwrap();
        let user = base.path().join("user");
        let bundled = base.path().join("bundled");
        std::fs::create_dir_all(user.join("shared")).unwrap();
        std::fs::write(
            user.join("shared/SKILL.md"),
            "---\nname: shared\ndescription: user\n---\n",
        )
        .unwrap();
        std::fs::create_dir_all(bundled.join("shared")).unwrap();
        std::fs::write(
            bundled.join("shared/SKILL.md"),
            "---\nname: shared\ndescription: bundled\n---\n",
        )
        .unwrap();

        let mut c = load_fixture(&user, &bundled);
        assert_eq!(c.detail("shared").unwrap().description, "user");

        c.unregister("shared").unwrap();
        assert_eq!(c.detail("shared").unwrap().description, "bundled");
        assert!(!c.catalog().iter().find(|e| e.id == "shared").unwrap().removable);
    }

    #[test]
    fn by_ids_filters_in_registration_order() {
        let mut c = LocalSkillsInner::load(Vec::<SkillDirSource>::new()).unwrap();
        c.register(sample_skill("b")).unwrap();
        c.register(sample_skill("a")).unwrap();
        c.register(sample_skill("c")).unwrap();
        let ids = vec!["a".into(), "c".into(), "missing".into()];
        let names: Vec<_> = c
            .collect(Some(&ids))
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["a", "c"]);
    }
}
