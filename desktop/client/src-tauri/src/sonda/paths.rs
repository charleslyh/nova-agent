//! Desktop 运行时目录布局：app data 下的 catalog 文件名、[`SondaRuntimePaths`]、种子数据与目录初始化。
//!
//! 不依赖 Tauri；测试与 [`crate::bundle`] 均构造本模块的类型。

use std::path::{Path, PathBuf};

/// App data 下的 settings 文件名。
pub const SETTINGS_FILE_NAME: &str = "settings.toml";
/// App data 下的 session catalog 文件名。
pub const SESSIONS_CATALOG_FILE_NAME: &str = "sessions.toml";
/// App data 下的 channel catalog 文件名。
pub const CHANNELS_CATALOG_FILE_NAME: &str = "channels.toml";
/// App data 下的工具 manifest 文件名。
pub const TOOLS_CATALOG_FILE_NAME: &str = "tools.toml";
/// 各 session transcript 子目录名。
pub const SESSIONS_DIR_NAME: &str = "sessions";
/// 用户安装 skills 子目录名。
pub const SKILLS_DIR_NAME: &str = "skills";

/// 桌面运行时所需的全部持久化路径（由 [`crate::bundle::resolve_runtime_paths`] 或测试夹具填充）。
#[derive(Debug, Clone)]
pub struct SondaRuntimePaths {
    pub skills_dir_bundled: PathBuf,
    pub skills_dir_user: PathBuf,
    pub settings_path_bundled: PathBuf,
    pub settings_path_user: PathBuf,
    pub sessions_catalog_path: PathBuf,
    pub channels_catalog_path: PathBuf,
    pub sessions_dir: PathBuf,
    pub tools_catalog_path: PathBuf,
    pub cli_path: PathBuf,
}

impl SondaRuntimePaths {
    /// App data 根目录（settings / catalogs 的父目录）。
    pub fn data_dir(&self) -> &Path {
        self.settings_path_user
            .parent()
            .expect("settings_path_user has parent")
    }
}

/// 创建 writable 目录，并确保 catalog 文件父目录存在。
pub(crate) fn ensure_layout(paths: &SondaRuntimePaths) -> std::io::Result<()> {
    std::fs::create_dir_all(&paths.skills_dir_user)?;
    std::fs::create_dir_all(&paths.sessions_dir)?;

    for file_path in [
        &paths.settings_path_user,
        &paths.sessions_catalog_path,
        &paths.channels_catalog_path,
        &paths.tools_catalog_path,
    ] {
        if let Some(parent) = file_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
    }

    Ok(())
}

/// 若 `dest` 尚不存在，从 `default_src` 复制工具 manifest，返回最终路径。
pub fn materialize_tools_catalog(dest: &Path, default_src: &Path) -> std::io::Result<PathBuf> {
    if dest.is_file() {
        return Ok(dest.to_path_buf());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(default_src, dest)?;
    Ok(dest.to_path_buf())
}

/// 确保用户 skills 目录存在；若为空则从 bundled 目录种子复制（不覆盖已有项）。
pub fn ensure_user_skills_dir(user: &Path, bundled: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(user)?;
    if !dir_has_skill_manifests(user) {
        seed_skills_from_bundled(bundled, user)?;
    }
    Ok(user.to_path_buf())
}

/// 确保 sessions 工作目录存在。
pub fn ensure_sessions_dir(data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = data_dir.join(SESSIONS_DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn seed_skills_from_bundled(bundled: &Path, user: &Path) -> std::io::Result<()> {
    if !bundled.is_dir() {
        return Ok(());
    }

    for entry in std::fs::read_dir(bundled)? {
        let entry = entry?;
        let src = entry.path();
        if !src.is_dir() || !path_has_skill_manifest(&src) {
            continue;
        }
        let dest = user.join(entry.file_name());
        if dest.exists() {
            continue;
        }
        copy_dir_recursive(&src, &dest)?;
    }
    Ok(())
}

fn path_has_skill_manifest(dir: &Path) -> bool {
    dir.join("SKILL.md").is_file() || dir.join("SKILL.toml").is_file()
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dest.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

fn dir_has_skill_manifests(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let path = entry.path();
        path.is_dir() && path_has_skill_manifest(&path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_file_names_match_app_data_layout() {
        assert_eq!(SETTINGS_FILE_NAME, "settings.toml");
        assert_eq!(SESSIONS_CATALOG_FILE_NAME, "sessions.toml");
        assert_eq!(TOOLS_CATALOG_FILE_NAME, "tools.toml");
    }
}
