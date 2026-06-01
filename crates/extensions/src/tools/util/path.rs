use std::path::{Path, PathBuf};

/// Resolves a tool path against `cwd`: relative paths are joined, absolute paths are used as-is.
pub(crate) fn resolve_path(cwd: &Path, user_path: &str) -> PathBuf {
    let path = Path::new(user_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
