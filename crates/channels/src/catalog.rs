//! Channel catalog: IM connector instances bound to sessions.
//!
//! Callers open via [`ChannelCatalog::open`] with a full file path to `channels.toml`
//! and per-type [`ChannelCatalogOps`] registered at bootstrap (typically from `moray-extensions`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{
    require_nonempty_trimmed, FileIoError, InvalidContent, MissingReference, Result,
};
use crate::manager::ChannelEntry;

/// Merge incoming catalog `data` with on-disk secrets (e.g. preserve secrets when client resubmits redacted placeholders).
pub type ChannelDataMergeFn = Arc<dyn Fn(&Value, &Value) -> Value + Send + Sync>;

/// Mask secrets in catalog `data` for HTTP responses.
pub type ChannelDataRedactFn = Arc<dyn Fn(&Value) -> Value + Send + Sync>;

/// Per-channel-type catalog hooks (secret merge/redact), registered at application bootstrap.
#[derive(Clone)]
pub struct ChannelCatalogOps {
    pub merge: ChannelDataMergeFn,
    pub redact: ChannelDataRedactFn,
}

impl ChannelCatalogOps {
    pub fn new(merge: ChannelDataMergeFn, redact: ChannelDataRedactFn) -> Self {
        Self { merge, redact }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct ChannelsData {
    #[serde(default, rename = "channel")]
    entries: Vec<ChannelEntry>,
}

pub struct ChannelCatalog {
    pub file_path: PathBuf,
    ops: HashMap<String, ChannelCatalogOps>,
    data: RwLock<ChannelsData>,
}

impl std::fmt::Debug for ChannelCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelCatalog")
            .field("file_path", &self.file_path)
            .field("ops", &self.ops.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl ChannelCatalog {
    pub fn open(
        file_path: &Path,
        ops: HashMap<String, ChannelCatalogOps>,
    ) -> Result<ChannelCatalog> {
        let inner = if file_path.exists() {
            let raw = std::fs::read_to_string(file_path)
                .map_err(|source| FileIoError::new("read", file_path.to_path_buf(), source))?;
            let data: ChannelsData = toml::from_str(&raw).map_err(|e| {
                InvalidContent::new(format!("invalid channels file: {e}"))
            })?;
            validate_data(data)?
        } else {
            ChannelsData::default()
        };

        Ok(ChannelCatalog {
            file_path: file_path.to_path_buf(),
            ops,
            data: RwLock::new(inner),
        })
    }

    pub fn entries(&self) -> Vec<ChannelEntry> {
        self.data.read().entries.clone()
    }

    pub fn has_channel(&self, channel_id: &str) -> bool {
        self.data
            .read()
            .entries
            .iter()
            .any(|e| e.channel_id == channel_id)
    }

    pub fn get(&self, channel_id: &str) -> Result<ChannelEntry> {
        let channel_id = require_nonempty_trimmed(channel_id, "channel_id")?;
        self.data
            .read()
            .entries
            .iter()
            .find(|e| e.channel_id == channel_id)
            .cloned()
            .ok_or_else(|| {
                MissingReference::new(format!("no channel row for channel_id `{channel_id}`")).into()
            })
    }

    pub fn find_by_session_id(&self, session_id: &str) -> Option<ChannelEntry> {
        self.data
            .read()
            .entries
            .iter()
            .find(|e| e.session_id == session_id)
            .cloned()
    }

    pub fn add(&self, entry: ChannelEntry) -> Result<()> {
        let entry = validate_entry(entry)?;
        let mut inner = self.data.write();
        if inner.entries.iter().any(|e| e.channel_id == entry.channel_id) {
            return Err(
                InvalidContent::new(format!("duplicate channel_id `{}`", entry.channel_id)).into(),
            );
        }
        if inner.entries.iter().any(|e| e.session_id == entry.session_id) {
            return Err(
                InvalidContent::new(format!(
                    "session_id `{}` already bound to a channel",
                    entry.session_id
                ))
                .into(),
            );
        }
        inner.entries.push(entry);
        drop(inner);
        save(self)
    }

    pub fn remove(&self, channel_id: &str) -> Result<ChannelEntry> {
        let channel_id = require_nonempty_trimmed(channel_id, "channel_id")?;
        let mut inner = self.data.write();
        let idx = inner
            .entries
            .iter()
            .position(|e| e.channel_id == channel_id)
            .ok_or_else(|| {
                MissingReference::new(format!("no channel row for channel_id `{channel_id}`"))
            })?;
        let removed = inner.entries.remove(idx);
        drop(inner);
        save(self)?;
        Ok(removed)
    }

    pub fn upsert(&self, channel_id: &str, channel_type: &str, data: Value) -> Result<()> {
        let channel_id = require_nonempty_trimmed(channel_id, "channel_id")?;
        let channel_type = require_nonempty_trimmed(channel_type, "channel_type")?.to_lowercase();

        let mut inner = self.data.write();
        if let Some(existing) = inner.entries.iter().find(|e| e.channel_id == channel_id) {
            if existing.channel_type != channel_type {
                return Err(InvalidContent::new(format!(
                    "channel type mismatch: expected `{}`, got `{channel_type}`",
                    existing.channel_type
                ))
                .into());
            }
            let merged = merge_channel_data(self, &channel_type, &data, &existing.data);
            let session_id = existing.session_id.clone();
            if let Some(idx) = inner.entries.iter().position(|e| e.channel_id == channel_id) {
                inner.entries[idx].data = merged;
            }
            drop(inner);
            save(self)?;
            let _ = session_id;
            return Ok(());
        }

        Err(MissingReference::new(format!(
            "cannot upsert unknown channel_id `{channel_id}`; use create_channel"
        ))
        .into())
    }

    pub fn redact_data(&self, entry: &ChannelEntry) -> Value {
        self.ops
            .get(&entry.channel_type)
            .map(|ops| (ops.redact)(&entry.data))
            .unwrap_or_else(|| entry.data.clone())
    }
}

fn validate_data(mut inner: ChannelsData) -> Result<ChannelsData> {
    let mut seen_channel = HashSet::new();
    let mut seen_session = HashSet::new();
    let mut entries = Vec::new();
    for e in inner.entries.drain(..) {
        entries.push(validate_entry(e)?);
    }
    for e in &entries {
        if !seen_channel.insert(e.channel_id.clone()) {
            return Err(
                InvalidContent::new(format!("duplicate channel_id `{}`", e.channel_id)).into(),
            );
        }
        if !seen_session.insert(e.session_id.clone()) {
            return Err(InvalidContent::new(format!(
                "duplicate session_id `{}` in channel catalog",
                e.session_id
            ))
            .into());
        }
    }
    Ok(ChannelsData { entries })
}

fn validate_entry(entry: ChannelEntry) -> Result<ChannelEntry> {
    Ok(ChannelEntry {
        channel_id: require_nonempty_trimmed(&entry.channel_id, "channel_id")?,
        session_id: require_nonempty_trimmed(&entry.session_id, "session_id")?,
        channel_type: require_nonempty_trimmed(&entry.channel_type, "channel_type")?.to_lowercase(),
        data: entry.data,
    })
}

fn merge_channel_data(
    catalog: &ChannelCatalog,
    channel_type: &str,
    incoming: &Value,
    existing: &Value,
) -> Value {
    catalog
        .ops
        .get(channel_type)
        .map(|ops| (ops.merge)(incoming, existing))
        .unwrap_or_else(|| incoming.clone())
}

fn save(catalog: &ChannelCatalog) -> Result<()> {
    let path = catalog.file_path.as_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|source| FileIoError::new("mkdir", parent.to_path_buf(), source))?;
    }
    let inner = catalog.data.read();
    let to_write = ChannelsData {
        entries: inner.entries.clone(),
    };
    drop(inner);

    let s = toml::to_string_pretty(&to_write).map_err(|e| {
        InvalidContent::new(format!("invalid channels file: {e}"))
    })?;
    std::fs::write(path, s)
        .map_err(|source| FileIoError::new("write", path.to_path_buf(), source))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn open_missing_file_is_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("channels.toml");
        let catalog = ChannelCatalog::open(&path, HashMap::new()).unwrap();
        assert!(catalog.entries().is_empty());
    }
}
