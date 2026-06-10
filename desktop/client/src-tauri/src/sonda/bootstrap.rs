//! 从已解析的 [`SondaRuntimePaths`](super::paths::SondaRuntimePaths) 组装 [`Sonda`]。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use moray_extensions::channels::{qq, wecom};
use moray_extensions::tools::{
    FileReadTool, FileWriteTool, ImageCreateTool, ImageEditTool, ShellTool, WebSearchTool,
};
use moray_channels::{
    ChannelCatalog, ChannelCatalogError, ChannelCatalogOps, ChannelDataMergeFn,
    ChannelDataRedactFn, ChannelEntry, ChannelError, ChannelFactoryFn,
};
use moray_core::TypedTool;
use moray_extensions::completions::OpenAIChatCompletion;
use moray_skillhub::SkillHub;
use moray_sonda::{
    SessionCatalogError, SkillCenter, SkillDirKind, SkillDirSource, SkillFilterKind, Sonda,
    SondaBuilder, SondaCompletionRegistration, SondaError, SondaSessionCatalog,
    SondaSessionTranscripts, SondaSessionWorkspace, SondaSettingsStore, SondaSettingsStoreError,
    SondaToolCatalog, SondaToolCatalogError, SondaToolRegistration,
};
use tracing::{info, warn};

use super::paths::{ensure_layout, SondaRuntimePaths};

const ENV_CLI: &str = "CLI";
const ENV_TOOLS_CATALOG_PATH: &str = "MORAY_TOOLS_CATALOG_PATH";

#[derive(Debug, thiserror::Error)]
pub enum SondaBootstrapError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Sonda(#[from] SondaError),

    #[error(transparent)]
    Session(#[from] moray_core::MorayError),

    #[error(transparent)]
    ToolCatalog(#[from] SondaToolCatalogError),
}

impl From<SondaSettingsStoreError> for SondaBootstrapError {
    fn from(err: SondaSettingsStoreError) -> Self {
        Self::Sonda(err.into())
    }
}

impl From<SessionCatalogError> for SondaBootstrapError {
    fn from(err: SessionCatalogError) -> Self {
        Self::Sonda(err.into())
    }
}

impl From<ChannelCatalogError> for SondaBootstrapError {
    fn from(err: ChannelCatalogError) -> Self {
        Self::Sonda(err.into())
    }
}

fn cli_subprocess_envs(
    cli_path: impl AsRef<Path>,
    tools_catalog_path: impl AsRef<Path>,
) -> Vec<(String, String)> {
    vec![
        (
            ENV_CLI.into(),
            cli_path.as_ref().display().to_string(),
        ),
        (
            ENV_TOOLS_CATALOG_PATH.into(),
            tools_catalog_path.as_ref().display().to_string(),
        ),
    ]
}

fn create_skill_center(paths: &SondaRuntimePaths) -> Result<SkillCenter, SondaBootstrapError> {
    let dir_list = [
        paths.skills_dir_user.display().to_string(),
        paths.skills_dir_bundled.display().to_string(),
    ];
    info!(dirs = ?dir_list, "load skills begin");

    let center = SkillCenter::load([
        SkillDirSource::new(&paths.skills_dir_user, SkillDirKind::User),
        SkillDirSource::new(&paths.skills_dir_bundled, SkillDirKind::Bundled),
    ])?;

    let loaded = center.skills(SkillFilterKind::All);
    let summary = loaded
        .iter()
        .map(|s| format!("{}({})", s.name, s.version))
        .collect::<Vec<_>>()
        .join(", ");

    if loaded.is_empty() {
        warn!(
            dirs = ?dir_list,
            "load skills failed, no skills found; expected <dir>/<name>/SKILL.md child folders"
        );
    } else  {
        info!("load skills success, count={}, [{summary}]", loaded.len());
    }

    Ok(center)
}

fn completion_registration() -> SondaCompletionRegistration {
    SondaCompletionRegistration::new(|endpoint| Arc::new(OpenAIChatCompletion::new(endpoint)))
}

fn tool_registrations(
    cli_path: impl AsRef<Path>,
    tools_catalog_path: impl AsRef<Path>,
) -> Vec<SondaToolRegistration> {
    let cli_envs = cli_subprocess_envs(cli_path, tools_catalog_path);
    vec![
        SondaToolRegistration::new(FileReadTool::NAME, |session_dir| {
            Arc::new(FileReadTool::new(session_dir))
        }),
        SondaToolRegistration::new(FileWriteTool::NAME, |session_dir| {
            Arc::new(FileWriteTool::new(session_dir))
        }),
        SondaToolRegistration::new(WebSearchTool::NAME, |_| Arc::new(WebSearchTool)),
        SondaToolRegistration::new(ImageCreateTool::NAME, |_| Arc::new(ImageCreateTool)),
        SondaToolRegistration::new(ImageEditTool::NAME, |session_dir| {
            Arc::new(ImageEditTool::new(session_dir))
        }),
        SondaToolRegistration::new(ShellTool::NAME, move |session_dir| {
            Arc::new(ShellTool::new(session_dir, cli_envs.clone()))
        }),
    ]
}

fn create_channel_catalog_ops() -> HashMap<String, ChannelCatalogOps> {
    let mut map = HashMap::new();
    map.insert(
        "qq".to_string(),
        ChannelCatalogOps::new(
            Arc::new(qq::merge_secrets) as ChannelDataMergeFn,
            Arc::new(qq::redact_secrets) as ChannelDataRedactFn,
        ),
    );
    map.insert(
        "wecom".to_string(),
        ChannelCatalogOps::new(
            Arc::new(wecom::merge_secrets) as ChannelDataMergeFn,
            Arc::new(wecom::redact_secrets) as ChannelDataRedactFn,
        ),
    );
    map
}

fn channel_factories(workspace_dir: PathBuf) -> HashMap<String, ChannelFactoryFn> {
    let mut map = HashMap::new();
    map.insert(
        "qq".to_string(),
        Arc::new(|entry: &ChannelEntry| {
            qq::channel_from_config(&entry.data).map_err(|e| ChannelError::op(e.to_string()))
        }) as ChannelFactoryFn,
    );
    map.insert(
        "wecom".to_string(),
        Arc::new(move |entry: &ChannelEntry| {
            wecom::channel_from_config(&entry.data, workspace_dir.clone())
                .map_err(|e| ChannelError::op(e.to_string()))
        }) as ChannelFactoryFn,
    );
    map
}

pub fn build_sonda(
    paths: &SondaRuntimePaths,
    session_workspace: Arc<SondaSessionWorkspace>,
) -> Result<Sonda, SondaBootstrapError> {
    ensure_layout(paths)?;

    let settings_store = Arc::new(SondaSettingsStore::load(
        &paths.settings_path_bundled,
        &paths.settings_path_user,
    )?);
    let session_catalog = Arc::new(SondaSessionCatalog::open(&paths.sessions_catalog_path)?);
    let skill_center = create_skill_center(paths)?;
    let skill_hub = SkillHub::new(&paths.skills_dir_user);
    let channel_catalog = Arc::new(ChannelCatalog::open(
        &paths.channels_catalog_path,
        create_channel_catalog_ops(),
    )?);
    let session_transcripts = Arc::new(SondaSessionTranscripts::new(&paths.sessions_dir));
    let tool_catalog = SondaToolCatalog::open(&paths.tools_catalog_path)?;

    SondaBuilder::new()
        .settings(settings_store)
        .completion_registration(completion_registration())
        .skill_center(skill_center)
        .skill_hub(skill_hub)
        .session_catalog(session_catalog)
        .session_transcripts(session_transcripts)
        .channel_catalog(channel_catalog)
        .channel_factories(channel_factories(paths.sessions_dir.clone()))
        .harness_components(
            session_workspace,
            tool_catalog,
            tool_registrations(&paths.cli_path, &paths.tools_catalog_path),
        )
        .build()
        .map_err(Into::into)
}
