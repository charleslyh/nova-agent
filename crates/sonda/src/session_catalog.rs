//! Session catalog: known `session_id`s and optional per-session `agent_id` bindings.
//!
//! 调用方通过 [`SondaSessionCatalog::open`] 传入完整文件路径。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::error::{require_nonempty_trimmed, FileIoError, InvalidContent, MissingReference};

type Result<T> = std::result::Result<T, SessionCatalogError>;

/// `sessions.toml` 相关错误。
#[derive(Debug, Error)]
pub enum SessionCatalogError {
    #[error(transparent)]
    InvalidContent(#[from] InvalidContent),

    #[error(transparent)]
    MissingReference(#[from] MissingReference),

    #[error(transparent)]
    FileIo(#[from] FileIoError),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SessionsData {
    default_agent_id: String,

    #[serde(default)]
    entries: Vec<SessionCatalogEntry>,
}

#[derive(Debug)]
pub struct SondaSessionCatalog {
    pub file_path: PathBuf,
    data: RwLock<SessionsData>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionCatalogEntry {
    pub session_id: String,
    /// Display name; when omitted in `sessions.toml`, filled from `session_id` on load.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub agent_id: Option<String>,
}

/// First 16 Unicode scalar values of trimmed `input` (used as session display name).
pub(crate) fn session_name_from_input(input: &str) -> String {
    input.trim().chars().take(16).collect()
}

impl SondaSessionCatalog {
    pub fn open(file_path: &Path) -> Result<SondaSessionCatalog> {
        let raw = std::fs::read_to_string(file_path)
            .map_err(|source| FileIoError::new("read", file_path.to_path_buf(), source))?;
        let data: SessionsData = toml::from_str(&raw).map_err(|e| {
            InvalidContent::new(format!("invalid sessions file: {e}"))
        })?;
        let inner = validate_data(data)?;

        Ok(SondaSessionCatalog {
            file_path: file_path.to_path_buf(),
            data: RwLock::new(inner),
        })
    }

    pub fn default_agent_id(&self) -> String {
        self.data.read().default_agent_id.clone()
    }

    pub fn has_session(&self, session_id: &str) -> bool {
        self.data
            .read()
            .entries
            .iter()
            .any(|e| e.session_id == session_id)
    }

    pub fn entries(&self) -> Vec<SessionCatalogEntry> {
        self.data.read().entries.clone()
    }

    pub fn add_session_entry(&self, session_id: &str, name: &str) -> Result<String> {
        let session_id = require_nonempty_trimmed(session_id, "session_id")?;
        let name = require_nonempty_trimmed(&session_name_from_input(name), "name")?;
        let mut inner = self.data.write();
        if let Some(existing) = inner.entries.iter().find(|e| e.session_id == session_id) {
            return Ok(existing.name.clone());
        }
        inner.entries.push(SessionCatalogEntry {
            session_id,
            name: name.clone(),
            agent_id: None,
        });
        save_locked(self, &inner)?;
        Ok(name)
    }

    pub fn remove_session_entry(&self, session_id: &str) -> Result<()> {
        let session_id = require_nonempty_trimmed(session_id, "session_id")?;
        let mut inner = self.data.write();
        let before = inner.entries.len();
        inner.entries.retain(|e| e.session_id != session_id);
        if inner.entries.len() == before {
            return Err(
                MissingReference::new(format!("no [[entries]] row for session_id `{session_id}`"))
                    .into(),
            );
        }
        save_locked(self, &inner)?;
        Ok(())
    }

    pub fn get_session_agent_id(&self, session_id: &str) -> Result<String> {
        ensure_session_row(self, session_id)?;
        Ok(self.logical_agent_id(session_id))
    }

    /// Updates `[[entries]]` for `session_id` and persists `sessions.toml`.
    ///
    /// Caller must ensure `agent_id` references a valid agent in `server.toml` when applicable.
    pub fn set_session_agent_id(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<()> {
        let session_id = require_nonempty_trimmed(session_id, "session_id")?;
        let agent_id = require_nonempty_trimmed(agent_id, "agent_id")?;
        let mut inner = self.data.write();
        let default_agent_id = inner.default_agent_id.clone();
        let e = inner
            .entries
            .iter_mut()
            .find(|e| e.session_id == session_id)
            .ok_or_else(|| {
                MissingReference::new(format!("no [[entries]] row for session_id `{session_id}`"))
            })?;

        if agent_id == default_agent_id {
            e.agent_id = None;
        } else {
            e.agent_id = Some(agent_id);
        }
        save_locked(self, &inner)?;
        Ok(())
    }

    fn logical_agent_id(&self, session_id: &str) -> String {
        let inner = self.data.read();
        if let Some(e) = inner.entries.iter().find(|e| e.session_id == session_id) {
            if let Some(ref aid) = e.agent_id {
                return aid.clone();
            }
        }
        inner.default_agent_id.clone()
    }
}

fn validate_data(inner: SessionsData) -> Result<SessionsData> {
    let default_agent_id = require_nonempty_trimmed(&inner.default_agent_id, "default_agent_id")?;

    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for e in inner.entries {
        let session_id = require_nonempty_trimmed(&e.session_id, "entries.session_id")?;
        if !seen.insert(session_id.clone()) {
            return Err(
                InvalidContent::new(format!("duplicate entries session_id `{session_id}`")).into(),
            );
        }
        let name = match require_nonempty_trimmed(&e.name, "entries.name") {
            Ok(n) => n,
            Err(_) => session_name_from_input(&session_id),
        };
        let agent_id = match e.agent_id {
            None => None,
            Some(s) => Some(require_nonempty_trimmed(&s, "entries.agent_id")?),
        };
        entries.push(SessionCatalogEntry {
            session_id,
            name,
            agent_id,
        });
    }

    Ok(SessionsData {
        default_agent_id,
        entries,
    })
}

/// Rows omitted on save: only those whose `agent_id` explicitly equals `default_agent_id` (redundant).
/// Rows with no `agent_id` are kept so pinned `session_id`s remain writable by [`SondaSessionCatalog::set_session_agent_id`].
fn entry_persists_on_disk(entry: &SessionCatalogEntry, default_agent_id: &str) -> bool {
    match entry.agent_id.as_deref() {
        None => true,
        Some(a) if a == default_agent_id => false,
        Some(_) => true,
    }
}

fn save_locked(catalog: &SondaSessionCatalog, inner: &SessionsData) -> Result<()> {
    let path = catalog.file_path.as_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| FileIoError::new("mkdir", parent.to_path_buf(), source))?;
    }
    let to_write = SessionsData {
        default_agent_id: inner.default_agent_id.clone(),
        entries: inner
            .entries
            .iter()
            .filter(|e| entry_persists_on_disk(e, inner.default_agent_id.as_str()))
            .cloned()
            .collect(),
    };

    let s = toml::to_string_pretty(&to_write).map_err(|e| {
        InvalidContent::new(format!("invalid sessions file: {e}"))
    })?;
    std::fs::write(path, s)
        .map_err(|source| FileIoError::new("write", path.to_path_buf(), source))?;
    Ok(())
}

fn ensure_session_row(catalog: &SondaSessionCatalog, session_id: &str) -> Result<()> {
    if !catalog.has_session(session_id) {
        return Err(
            MissingReference::new(format!("no [[entries]] row for session_id `{session_id}`"))
                .into(),
        );
    }
    Ok(())
}
