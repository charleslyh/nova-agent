//! Tauri 打包资源与 app data 路径解析 → [`SondaRuntimePaths`](crate::sonda::SondaRuntimePaths)。

use std::path::{Path, PathBuf};

use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};

use crate::sonda::{
    ensure_sessions_dir, ensure_user_skills_dir, materialize_sessions_catalog,
    materialize_tools_catalog, SondaRuntimePaths, CHANNELS_CATALOG_FILE_NAME,
    SESSIONS_CATALOG_FILE_NAME, SETTINGS_FILE_NAME, SKILLS_DIR_NAME, TOOLS_CATALOG_FILE_NAME,
};

const MORAY_CLI_EXTERNAL_BIN: &str = "resources/binaries/moray-cli";
const COMPILED_TOOLS_CATALOG_SRC: Option<&str> = option_env!("MORAY_TOOLS_CATALOG_SRC");
const COMPILED_SESSIONS_CATALOG_SRC: Option<&str> = option_env!("MORAY_SESSIONS_CATALOG_SRC");
const COMPILED_SKILLS_SRC: Option<&str> = option_env!("MORAY_SKILLS_SRC_DIR");
const COMPILED_SETTINGS_SRC: Option<&str> = option_env!("MORAY_SETTINGS_SRC");
const MORAY_CLI_RUNTIME_NAME: &str = "moray-cli";
const SKILLS_RUNTIME_DIR: &str = "skills";

/// 解析桌面运行时所需的全部路径（Tauri app data + bundle 资源）。
pub fn resolve_runtime_paths(app: &AppHandle) -> Result<SondaRuntimePaths, String> {
    let data_dir = resolve_data_dir(app)?;
    let skills_dir_bundled = resolve_skills_dir_bundled(app)?;
    let skills_dir_user = ensure_user_skills_dir(&data_dir.join(SKILLS_DIR_NAME))
        .map_err(|e| format!("user skills dir: {e}"))?;
    let tools_catalog_path = {
        let dest = data_dir.join(TOOLS_CATALOG_FILE_NAME);
        let default = locate_default_tools_catalog(app)?;
        materialize_tools_catalog(&dest, &default)
            .map_err(|e| format!("tools catalog: {e}"))?
    };
    let sessions_dir = ensure_sessions_dir(&data_dir).map_err(|e| format!("sessions dir: {e}"))?;
    let sessions_catalog_path = {
        let dest = data_dir.join(SESSIONS_CATALOG_FILE_NAME);
        let default = locate_default_sessions_catalog(app)?;
        materialize_sessions_catalog(&dest, &default)
            .map_err(|e| format!("sessions catalog: {e}"))?
    };
    let settings_path_bundled = locate_default_settings(app)?;
    let settings_path_user = data_dir.join(SETTINGS_FILE_NAME);

    Ok(SondaRuntimePaths {
        cli_path: resolve_moray_cli_path()?,
        skills_dir_bundled,
        skills_dir_user,
        tools_catalog_path,
        sessions_dir,
        sessions_catalog_path,
        channels_catalog_path: data_dir.join(CHANNELS_CATALOG_FILE_NAME),
        settings_path_bundled,
        settings_path_user,
    })
}

fn resolve_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("create app data dir: {e}"))?;
    Ok(data_dir)
}

fn locate_bundled_resource(
    app: &AppHandle,
    file_name: &str,
    compiled_src: Option<&str>,
    label: &str,
    setup_hint: &str,
) -> Result<PathBuf, String> {
    let mut tried = Vec::new();

    if let Ok(path) = path_next_to_executable(file_name) {
        tried.push(path.display().to_string());
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Ok(resource_base) = app.path().resource_dir() {
        let path = resource_base.join(file_name);
        tried.push(path.display().to_string());
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Ok(path) = app.path().resolve(file_name, BaseDirectory::Resource) {
        tried.push(path.display().to_string());
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Some(src) = compiled_src {
        let path = PathBuf::from(src);
        tried.push(path.display().to_string());
        if path.is_file() {
            return Ok(path);
        }
    }

    Err(format!(
        "{label} not found. Tried:\n  {}\n{setup_hint}",
        tried.join("\n  ")
    ))
}

fn locate_default_settings(app: &AppHandle) -> Result<PathBuf, String> {
    locate_bundled_resource(
        app,
        SETTINGS_FILE_NAME,
        COMPILED_SETTINGS_SRC,
        "settings bundle",
        "Ensure src-tauri/resources/settings.toml exists and bundle.resources maps it to settings.toml.",
    )
}

fn locate_default_tools_catalog(app: &AppHandle) -> Result<PathBuf, String> {
    locate_bundled_resource(
        app,
        TOOLS_CATALOG_FILE_NAME,
        COMPILED_TOOLS_CATALOG_SRC,
        "tools catalog",
        "Ensure src-tauri/resources/tools.toml exists and bundle.resources maps it to tools.toml.",
    )
}

fn locate_default_sessions_catalog(app: &AppHandle) -> Result<PathBuf, String> {
    locate_bundled_resource(
        app,
        SESSIONS_CATALOG_FILE_NAME,
        COMPILED_SESSIONS_CATALOG_SRC,
        "sessions catalog",
        "Ensure src-tauri/resources/sessions.toml exists and bundle.resources maps it to sessions.toml.",
    )
}

fn resolve_moray_cli_path() -> Result<PathBuf, String> {
    let path = path_next_to_executable(MORAY_CLI_RUNTIME_NAME)?;
    if path.is_file() {
        return Ok(path);
    }

    Err(format!(
        "moray-cli sidecar not found at {}\n\
         Install the CLI to src-tauri/{}/$(rustc --print host-tuple) \
         (see resources/binaries/README.md), then rebuild the desktop client.",
        path.display(),
        MORAY_CLI_EXTERNAL_BIN
    ))
}

fn resolve_skills_dir_bundled(app: &AppHandle) -> Result<PathBuf, String> {
    let mut tried = Vec::new();

    #[cfg(debug_assertions)]
    if let Some(path) = compiled_skills_src_if_valid(&mut tried) {
        return Ok(path);
    }

    match path_next_to_executable(SKILLS_RUNTIME_DIR) {
        Ok(path) => {
            tried.push(path.display().to_string());
            if dir_has_skill_manifests(&path) {
                return Ok(path);
            }
        }
        Err(e) => tried.push(format!("{SKILLS_RUNTIME_DIR} ({e})")),
    }

    if let Ok(resource_base) = app.path().resource_dir() {
        let path = resource_base.join(SKILLS_RUNTIME_DIR);
        tried.push(path.display().to_string());
        if dir_has_skill_manifests(&path) {
            return Ok(path);
        }
    } else {
        tried.push("resource_dir (unavailable)".into());
    }

    match app.path().resolve(SKILLS_RUNTIME_DIR, BaseDirectory::Resource) {
        Ok(path) => {
            tried.push(path.display().to_string());
            if dir_has_skill_manifests(&path) {
                return Ok(path);
            }
        }
        Err(e) => tried.push(format!("resolve:{SKILLS_RUNTIME_DIR} ({e})")),
    }

    #[cfg(not(debug_assertions))]
    if let Some(path) = compiled_skills_src_if_valid(&mut tried) {
        return Ok(path);
    }

    Err(format!(
        "skills bundle not found. Tried:\n  {}\n\
         Ensure `src-tauri/resources/skills/<name>/SKILL.md` exists and run a full rebuild \
         (`cargo tauri dev` or `cargo build -p moray-desktop-client`).",
        tried.join("\n  ")
    ))
}

fn path_next_to_executable(relative: &str) -> Result<PathBuf, String> {
    let exe_path = tauri::utils::platform::current_exe().map_err(|e| e.to_string())?;
    let exe_dir = exe_path
        .parent()
        .ok_or_else(|| "application executable has no parent directory".to_string())?;

    let base_dir = if exe_dir.ends_with("deps") {
        exe_dir.parent().unwrap_or(exe_dir)
    } else {
        exe_dir
    };

    let mut path = base_dir.join(relative);

    #[cfg(windows)]
    {
        let already_exe = path.extension().is_some_and(|ext| ext == "exe");
        if !already_exe {
            path.as_mut_os_string().push(".exe");
        }
    }

    #[cfg(not(windows))]
    {
        if path.extension().is_some_and(|ext| ext == "exe") {
            path.set_extension("");
        }
    }

    Ok(path)
}

fn compiled_skills_src_if_valid(tried: &mut Vec<String>) -> Option<PathBuf> {
    let src = COMPILED_SKILLS_SRC?;
    let path = PathBuf::from(src);
    tried.push(path.display().to_string());
    dir_has_skill_manifests(&path).then_some(path)
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
        path.is_dir()
            && (path.join("SKILL.md").is_file() || path.join("SKILL.toml").is_file())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_runtime_name_is_flattened_basename() {
        assert_eq!(MORAY_CLI_RUNTIME_NAME, "moray-cli");
    }

    #[test]
    fn skills_runtime_dir_is_bundle_root_skills() {
        assert_eq!(SKILLS_RUNTIME_DIR, "skills");
    }

    #[test]
    fn compiled_skills_src_points_at_resources_skills() {
        let Some(src) = COMPILED_SKILLS_SRC else {
            return;
        };
        let path = PathBuf::from(src);
        assert!(
            path.ends_with("resources/skills") || path.ends_with("skills"),
            "expected MORAY_SKILLS_SRC_DIR to end with resources/skills, got {}",
            path.display()
        );
    }
}
