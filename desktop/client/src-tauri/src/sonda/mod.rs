//! 桌面 [`Sonda`] 组装：路径布局（与 Tauri 无关）与 `build_sonda`。

mod bootstrap;
mod paths;

pub use bootstrap::build_sonda;
pub use paths::{
    materialize_tools_catalog, ensure_sessions_dir, ensure_user_skills_dir, SondaRuntimePaths,
    CHANNELS_CATALOG_FILE_NAME, SESSIONS_CATALOG_FILE_NAME, SESSIONS_DIR_NAME,
    SETTINGS_FILE_NAME, TOOLS_CATALOG_FILE_NAME,
};

pub(crate) use paths::SKILLS_DIR_NAME;
