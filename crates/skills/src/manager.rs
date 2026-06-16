//! [`SkillsManager`] — local catalog + SkillHub with install/uninstall orchestration.

use std::path::Path;

use serde::Serialize;

use crate::error::{InstallSkillError, Result, UninstallSkillError};
use crate::hub::SkillHub;
use crate::local::LocalSkills;

/// Outcome of uninstalling a user skill (files removed from disk).
#[derive(Debug, Clone, Serialize)]
pub struct UninstallSkillResult {
    pub slug: String,
}

/// Entry point for application skills: local catalog, marketplace client, lifecycle APIs.
#[derive(Clone)]
pub struct SkillsManager {
    local: LocalSkills,
    hub: SkillHub,
}

impl SkillsManager {
    /// `user_dir`: user-writable root (Hub download target). `bundled_dir`: built-in read-only skills.
    pub fn load(user_dir: impl AsRef<Path>, bundled_dir: impl AsRef<Path>) -> Result<Self> {
        let user_dir = user_dir.as_ref();
        Ok(Self {
            local: LocalSkills::load(user_dir, bundled_dir)?,
            hub: SkillHub::new(user_dir),
        })
    }

    pub fn local(&self) -> &LocalSkills {
        &self.local
    }

    pub fn hub(&self) -> &SkillHub {
        &self.hub
    }

    /// Download from SkillHub and register in the local catalog. Rolls back on-disk files if registration fails.
    pub async fn install(
        &self,
        slug: &str,
        force: bool,
    ) -> std::result::Result<(), InstallSkillError> {
        let download = self.hub.download(slug, force).await?;
        let downloaded_dir = download.downloaded_dir;
        if let Err(e) = self.local.register_dir(&downloaded_dir) {
            if downloaded_dir.exists() {
                let _ = std::fs::remove_dir_all(&downloaded_dir);
            }
            return Err(InstallSkillError::RegisterFailed(e.to_string()));
        }
        Ok(())
    }

    /// Unregister from the local catalog and remove the user skill directory from disk.
    pub fn uninstall(
        &self,
        skill_id: &str,
    ) -> std::result::Result<UninstallSkillResult, UninstallSkillError> {
        let unregistered = self.local.unregister(skill_id)?;
        if unregistered.user_dir.exists() {
            std::fs::remove_dir_all(&unregistered.user_dir).map_err(|e| {
                UninstallSkillError::RemoveFiles(format!(
                    "{}: {e}",
                    unregistered.user_dir.display()
                ))
            })?;
        }
        Ok(UninstallSkillResult {
            slug: unregistered.slug,
        })
    }
}
