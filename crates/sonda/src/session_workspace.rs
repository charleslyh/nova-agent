//! Per-session on-disk workspace paths, directory listing, and upload staging.

use std::fs;
use std::path::{Component, Path, PathBuf};

use moray_session::TurnResource;
use serde::Serialize;
use uuid::Uuid;

use crate::error::{FileIoError, InvalidContent, Result};

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

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

    /// Copy picker paths into `{session_dir}/uploads/` and return staged [`TurnResource`] rows.
    pub async fn stage_session_images(
        &self,
        session_id: &str,
        source_paths: Vec<String>,
    ) -> Result<Vec<TurnResource>> {
        if source_paths.is_empty() {
            return Ok(Vec::new());
        }

        let session_dir = self.session_dir(session_id);
        let uploads_dir = session_dir.join("uploads");
        let session_id = session_id.to_string();

        tokio::task::spawn_blocking(move || {
            stage_session_images_blocking(
                session_id,
                session_dir,
                uploads_dir,
                source_paths,
            )
        })
        .await
        .map_err(|err| {
            InvalidContent::new(format!("stage_session_images task failed: {err}"))
        })?
    }

    /// Recursively list files under the session workspace (depth and node caps apply).
    pub fn list_session_tree(&self, session_id: &str) -> Result<SessionWorkspaceTree> {
        let root = self.session_dir(session_id);
        let path_str = root.to_string_lossy().to_string();
        let mut remaining_nodes = TREE_MAX_NODES;
        let entries = list_entries_recursive(&root, 0, &mut remaining_nodes)?;

        Ok(SessionWorkspaceTree {
            path: path_str,
            entries,
        })
    }
}

fn stage_session_images_blocking(
    session_id: String,
    session_dir: PathBuf,
    uploads_dir: PathBuf,
    source_paths: Vec<String>,
) -> Result<Vec<TurnResource>> {
    if !session_dir.is_dir() {
        return Err(InvalidContent::new(format!("unknown session: {session_id}")).into());
    }

    fs::create_dir_all(&uploads_dir)
        .map_err(|source| FileIoError::new("create uploads dir", uploads_dir.clone(), source))?;

    let allowed_roots = allowed_stage_source_roots(&session_dir)?;

    let mut staged = Vec::with_capacity(source_paths.len());
    for source in source_paths {
        let src = resolve_stage_source_path(&source, &allowed_roots)?;
        if !is_allowed_image(&src) {
            return Err(InvalidContent::new(format!("unsupported image type: {source}")).into());
        }
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| !e.is_empty())
            .unwrap_or("bin");
        let dest = uploads_dir.join(format!("{}.{}", Uuid::new_v4(), ext));
        fs::copy(&src, &dest).map_err(|source| {
            FileIoError::new("copy image into session uploads", dest.clone(), source)
        })?;
        staged.push(TurnResource::Image {
            path: dest.to_string_lossy().into_owned(),
        });
    }

    Ok(staged)
}

fn allowed_stage_source_roots(session_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    if session_dir.is_dir() {
        roots.push(
            fs::canonicalize(session_dir).map_err(|_| {
                InvalidContent::new(format!(
                    "session workspace not accessible: {}",
                    session_dir.display()
                ))
            })?,
        );
    }
    if let Some(home) = std::env::home_dir() {
        if let Ok(canonical) = fs::canonicalize(&home) {
            roots.push(canonical);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let volumes = Path::new("/Volumes");
        if volumes.is_dir() {
            if let Ok(canonical) = fs::canonicalize(volumes) {
                roots.push(canonical);
            }
        }
    }
    if roots.is_empty() {
        return Err(InvalidContent::new("no allowed stage source roots configured").into());
    }
    Ok(roots)
}

fn is_under_allowed_roots(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

fn resolve_stage_source_path(source: &str, allowed_roots: &[PathBuf]) -> Result<PathBuf> {
    let source = source.trim();
    if source.is_empty() {
        return Err(InvalidContent::new("empty source path").into());
    }
    let raw = Path::new(source);
    for component in raw.components() {
        if matches!(component, Component::ParentDir) {
            return Err(InvalidContent::new(format!("invalid source path: {source}")).into());
        }
    }
    let canonical = fs::canonicalize(raw).map_err(|_| {
        InvalidContent::new(format!("source path not accessible: {source}"))
    })?;
    if !canonical.is_file() {
        return Err(InvalidContent::new(format!("not a file: {source}")).into());
    }
    if !is_under_allowed_roots(&canonical, allowed_roots) {
        return Err(
            InvalidContent::new(format!("source path outside allowed scope: {source}")).into(),
        );
    }
    Ok(canonical)
}

fn is_allowed_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn should_skip_entry(name: &str) -> bool {
    name == ".DS_Store" || name == "transcript.jsonl"
}

fn list_entries_recursive(
    dir: &Path,
    depth: usize,
    remaining_nodes: &mut usize,
) -> Result<Vec<SessionWorkspaceEntry>> {
    if depth > TREE_MAX_DEPTH || *remaining_nodes == 0 {
        return Ok(Vec::new());
    }

    let read_dir = match fs::read_dir(dir) {
        Ok(read_dir) => read_dir,
        Err(source) => {
            if depth == 0 {
                return Err(
                    FileIoError::new("read session workspace", dir.to_path_buf(), source).into(),
                );
            }
            tracing::warn!(
                path = %dir.display(),
                error = %source,
                "session workspace: skip unreadable directory"
            );
            return Ok(Vec::new());
        }
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
            list_entries_recursive(&path_buf, depth + 1, remaining_nodes)?
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
    Ok(entries)
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
    fn list_session_tree_errors_when_root_missing() {
        let dir = tempdir().unwrap();
        let workspace = SondaSessionWorkspace::new(dir.path());
        assert!(workspace.list_session_tree("no-such-session").is_err());
    }

    #[test]
    fn allows_common_image_extensions() {
        assert!(is_allowed_image(Path::new("/tmp/a.PNG")));
        assert!(is_allowed_image(Path::new("/tmp/photo.jpeg")));
        assert!(!is_allowed_image(Path::new("/tmp/doc.pdf")));
    }

    #[test]
    fn resolve_stage_source_path_rejects_paths_outside_allowed_roots() {
        let dir = tempdir().unwrap();
        let session_dir = dir.path().join("sess-1");
        fs::create_dir_all(&session_dir).unwrap();
        let allowed = allowed_stage_source_roots(&session_dir).unwrap();
        let outside = Path::new("/etc/hosts");
        if outside.is_file() {
            assert!(resolve_stage_source_path(
                outside.to_string_lossy().as_ref(),
                &allowed
            )
            .is_err());
        }
    }

    #[test]
    fn resolve_stage_source_path_accepts_file_under_session_dir() {
        let dir = tempdir().unwrap();
        let session_dir = dir.path().join("sess-1");
        fs::create_dir_all(&session_dir).unwrap();
        let image = session_dir.join("photo.png");
        fs::write(&image, b"png").unwrap();
        let allowed = allowed_stage_source_roots(&session_dir).unwrap();
        let resolved =
            resolve_stage_source_path(image.to_string_lossy().as_ref(), &allowed).unwrap();
        assert!(resolved.is_file());
        assert!(is_allowed_image(&resolved));
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
