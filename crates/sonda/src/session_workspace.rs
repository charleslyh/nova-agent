//! Per-session on-disk workspace paths and directory listing.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{FileIoError, Result};

const TREE_MAX_DEPTH: usize = 8;
const TREE_MAX_NODES: usize = 2000;

/// Session disk workspace root: `{sessions_root}/{session_id}/`.
#[derive(Debug, Clone)]
pub struct SondaSessionWorkspace {
    sessions_root: PathBuf,
}

/// One node in a session workspace tree (`GET /sessions/{id}/workspace`).
#[derive(Debug, Clone, Serialize)]
pub struct SessionWorkspaceEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<SessionWorkspaceEntry>,
}

/// Listing result for a single session workspace directory.
#[derive(Debug, Clone, Serialize)]
pub struct SessionWorkspaceTree {
    pub path: String,
    pub entries: Vec<SessionWorkspaceEntry>,
}

/// Path-only response (`GET /sessions/{id}/workspace/path`).
#[derive(Debug, Clone, Serialize)]
pub struct SessionWorkspacePath {
    pub path: String,
}

impl SondaSessionWorkspace {
    pub fn new(sessions_root: impl Into<PathBuf>) -> Self {
        Self {
            sessions_root: sessions_root.into(),
        }
    }

    pub fn sessions_root(&self) -> &Path {
        &self.sessions_root
    }

    /// `{sessions_root}/{session_id}`.
    pub fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_root.join(session_id)
    }

    /// Absolute session workspace path for APIs and clients (`GET .../workspace/path`).
    pub fn session_workspace_path(&self, session_id: &str) -> SessionWorkspacePath {
        SessionWorkspacePath {
            path: self.session_dir(session_id).to_string_lossy().to_string(),
        }
    }

    /// Recursively list files under the session workspace (depth and node caps apply).
    pub fn list_session_tree(&self, session_id: &str) -> Result<SessionWorkspaceTree> {
        let root = self.session_dir(session_id);
        let path_str = root.to_string_lossy().to_string();

        if let Err(source) = fs::read_dir(&root) {
            return Err(
                FileIoError::new("read session workspace", root.clone(), source).into(),
            );
        }

        let mut remaining_nodes = TREE_MAX_NODES;
        let entries = list_entries_recursive(&root, 0, &mut remaining_nodes);

        Ok(SessionWorkspaceTree {
            path: path_str,
            entries,
        })
    }
}

fn should_skip_entry(name: &str) -> bool {
    name == ".DS_Store" || name == "transcript.jsonl"
}

fn list_entries_recursive(
    dir: &Path,
    depth: usize,
    remaining_nodes: &mut usize,
) -> Vec<SessionWorkspaceEntry> {
    if depth > TREE_MAX_DEPTH || *remaining_nodes == 0 {
        return Vec::new();
    }

    let Ok(read_dir) = fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        if *remaining_nodes == 0 {
            break;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if should_skip_entry(&name) {
            continue;
        }

        let path_buf = entry.path();
        let path = path_buf.to_string_lossy().to_string();
        let file_type = match entry.file_type() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let is_symlink = file_type.is_symlink();
        let is_dir = file_type.is_dir();

        *remaining_nodes -= 1;

        let children = if is_dir && !is_symlink {
            list_entries_recursive(&path_buf, depth + 1, remaining_nodes)
        } else {
            Vec::new()
        };

        entries.push(SessionWorkspaceEntry {
            name,
            path,
            is_dir,
            children,
        });
    }

    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn session_workspace_path_joins_sessions_root() {
        let dir = tempdir().unwrap();
        let workspace = SondaSessionWorkspace::new(dir.path());
        let path = workspace.session_workspace_path("abc-123");
        assert!(path.path.ends_with("abc-123"));
        assert!(path.path.starts_with(dir.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn list_session_tree_skips_transcript_and_lists_files() {
        let dir = tempdir().unwrap();
        let workspace = SondaSessionWorkspace::new(dir.path());
        let session_dir = workspace.session_dir("sess-1");
        fs::create_dir_all(&session_dir).unwrap();
        fs::write(session_dir.join("transcript.jsonl"), "x").unwrap();
        fs::write(session_dir.join("note.txt"), "hi").unwrap();
        fs::create_dir_all(session_dir.join("out")).unwrap();
        fs::write(session_dir.join("out/a.md"), "#").unwrap();

        let tree = workspace.list_session_tree("sess-1").unwrap();
        assert!(tree.path.ends_with("sess-1"));
        let names: Vec<_> = tree.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(!names.contains(&"transcript.jsonl"));
        assert!(names.contains(&"note.txt"));
        assert!(names.contains(&"out"));
    }
}
