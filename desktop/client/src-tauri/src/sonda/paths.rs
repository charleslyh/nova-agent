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

/// 若 `dest` 尚不存在，从 bundle 默认文件复制到 app data，返回最终路径。
pub fn materialize_initial_file(dest: &Path, default_src: &Path) -> std::io::Result<PathBuf> {
    if dest.is_file() {
        return Ok(dest.to_path_buf());
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(default_src, dest)?;
    Ok(dest.to_path_buf())
}

/// 若 `dest` 尚不存在，从 `default_src` 复制工具 manifest，返回最终路径。
pub fn materialize_tools_catalog(dest: &Path, default_src: &Path) -> std::io::Result<PathBuf> {
    materialize_initial_file(dest, default_src)
}

/// 若 `dest` 尚不存在，从 `default_src` 复制 session catalog，返回最终路径。
pub fn materialize_sessions_catalog(dest: &Path, default_src: &Path) -> std::io::Result<PathBuf> {
    materialize_initial_file(dest, default_src)
}

/// 确保用户 skills 目录存在（仅用于 SkillHub 安装等可写 skill；不复制 bundled skills）。
pub fn ensure_user_skills_dir(user: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(user)?;
    Ok(user.to_path_buf())
}

/// 确保 sessions 工作目录存在。
pub fn ensure_sessions_dir(data_dir: &Path) -> std::io::Result<PathBuf> {
    let dir = data_dir.join(SESSIONS_DIR_NAME);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
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

    #[test]
    fn materialize_sessions_catalog_copies_when_missing() {
        let base = std::env::temp_dir().join(format!(
            "moray-sessions-catalog-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let default_src = base.join("default-sessions.toml");
        let dest = base.join("sessions.toml");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(&default_src, "default_agent_id = \"abc\"\n").unwrap();

        let path = materialize_sessions_catalog(&dest, &default_src).unwrap();
        assert_eq!(path, dest);
        assert!(dest.is_file());
        assert_eq!(
            std::fs::read_to_string(&dest).unwrap(),
            "default_agent_id = \"abc\"\n"
        );

        std::fs::write(&dest, "default_agent_id = \"user\"\n").unwrap();
        let path = materialize_sessions_catalog(&dest, &default_src).unwrap();
        assert_eq!(path, dest);
        assert_eq!(
            std::fs::read_to_string(&dest).unwrap(),
            "default_agent_id = \"user\"\n"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ensure_user_skills_dir_does_not_copy_bundled_skills() {
        let base = std::env::temp_dir().join(format!(
            "moray-skills-dir-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        let bundled = base.join("bundled");
        let user = base.join("user");
        std::fs::create_dir_all(bundled.join("web-fetch")).unwrap();
        std::fs::write(
            bundled.join("web-fetch/SKILL.md"),
            "---\nname: web-fetch\ndescription: test\n---\n",
        )
        .unwrap();

        ensure_user_skills_dir(&user).unwrap();
        assert!(user.is_dir());
        assert!(
            !user.join("web-fetch").exists(),
            "bundled skills must not be copied into the user skills dir"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
