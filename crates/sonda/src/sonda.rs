//! Assembled [`Sonda`] application and [`SondaBuilder`] bootstrap.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    SkillCenter, SondaAgentRunner, SondaSessionCatalog, SondaSessionFactory,
    SondaSessionTranscripts, SondaSessionWorkspace, SondaSettingsStore, SondaSnapshot,
    SondaToolCatalog, SondaToolRegistration, UnregisterSkillError,
};
use moray_skillhub::{SkillHub, SkillHubError};
use serde::Serialize;
use serde_json::Value;
use moray_core::ToolCallAuthorizer;
use moray_session::LiveSessions;

use crate::error::{
    require_nonempty_trimmed, InvalidArguments, InvalidContent, MissingReference, Result,
    SondaError,
};
use moray_channels::{ChannelCatalog, ChannelEntry, ChannelFactoryFn, ChannelsManager};
use moray_extensions::auths::AlwaysAsking;

#[derive(Debug, thiserror::Error)]
pub enum InstallSkillError {
    #[error(transparent)]
    Hub(#[from] SkillHubError),
    #[error("downloaded but catalog registration failed: {0}")]
    RegisterFailed(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct UninstallSkillResult {
    pub slug: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UninstallSkillError {
    #[error(transparent)]
    Unregister(#[from] UnregisterSkillError),
    #[error("failed to remove skill files: {0}")]
    RemoveFiles(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateChannelResult {
    pub channel_id: String,
    pub session_id: String,
}

/// Live application handle: settings, catalogs, transcripts, and in-process session runtimes.
///
/// **Write admission:** live `submit` requires a `sessions.toml` [[entries]] row (`UnknownSession` otherwise)
/// and a persisted transcript (created via [`Sonda::create_session`] or equivalent) via [`SondaSessionTranscripts`].
pub struct Sonda {
    pub settings_store: Arc<SondaSettingsStore>,
    pub skill_center: SkillCenter,
    pub session_catalog: Arc<SondaSessionCatalog>,
    pub session_transcripts: Arc<SondaSessionTranscripts>,
    pub session_workspace: Arc<SondaSessionWorkspace>,
    pub agent_runner: Arc<SondaAgentRunner>,
    pub skill_hub: SkillHub,
    pub authorizer: Arc<dyn ToolCallAuthorizer>,
    pub snapshot: Arc<SondaSnapshot>,
    pub live_sessions: LiveSessions,
    pub channel_catalog: Arc<ChannelCatalog>,
    pub channels: Arc<ChannelsManager>,
}

impl Sonda {
    #[allow(clippy::too_many_arguments)]
    fn new(
        settings_store: Arc<SondaSettingsStore>,
        skill_center: SkillCenter,
        session_catalog: Arc<SondaSessionCatalog>,
        session_transcripts: Arc<SondaSessionTranscripts>,
        session_workspace: Arc<SondaSessionWorkspace>,
        agent_runner: Arc<SondaAgentRunner>,
        skill_hub: SkillHub,
        authorizer: Arc<dyn ToolCallAuthorizer>,
        snapshot: Arc<SondaSnapshot>,
        live_sessions: LiveSessions,
        channel_catalog: Arc<ChannelCatalog>,
        channels: Arc<ChannelsManager>,
    ) -> Self {
        Self {
            settings_store,
            skill_center,
            session_catalog,
            session_transcripts,
            session_workspace,
            agent_runner,
            skill_hub,
            authorizer,
            snapshot,
            live_sessions,
            channel_catalog,
            channels,
        }
    }

    /// Download from SkillHub, register in [`SkillCenter`]. On registration failure, removes the downloaded directory.
    pub async fn install_skill(
        &self,
        slug: &str,
        force: bool,
    ) -> std::result::Result<(), InstallSkillError> {
        let download = self.skill_hub.download(slug, force).await?;
        let downloaded_dir = download.downloaded_dir;
        if let Err(e) = self.skill_center.register(&downloaded_dir) {
            if downloaded_dir.exists() {
                let _ = std::fs::remove_dir_all(&downloaded_dir);
            }
            return Err(InstallSkillError::RegisterFailed(e.to_string()));
        }
        Ok(())
    }

    /// Unregister from [`SkillCenter`] and remove the user skill directory from disk.
    pub fn uninstall_skill(
        &self,
        skill_id: &str,
    ) -> std::result::Result<UninstallSkillResult, UninstallSkillError> {
        let unregistered = self.skill_center.unregister(skill_id)?;
        if unregistered.user_dir.exists() {
            std::fs::remove_dir_all(&unregistered.user_dir).map_err(|e| {
                UninstallSkillError::RemoveFiles(format!(
                    "{}: {e}",
                    unregistered.user_dir.display()
                ))
            })?;
        }
        Ok(UninstallSkillResult {
            slug: unregistered.slug,
        })
    }

    pub fn update_agent(
        &self,
        agent_id: &str,
        name: &str,
        completion_id: &str,
        allowed_tools: Vec<String>,
        character: Option<String>,
    ) -> Result<()> {
        self.agent_runner.validate_allowed_tools(&allowed_tools)?;

        self.settings_store.update_agent(
            agent_id,
            name,
            completion_id,
            allowed_tools,
            character,
        )?;

        Ok(())
    }

    pub fn create_session(&self, name: &str) -> Result<String> {
        let session_id = new_session_id();
        self.init_session(&session_id, name)?;
        Ok(session_id)
    }

    pub fn create_channel(
        &self,
        name: &str,
        channel_type: &str,
        data: Value,
    ) -> Result<CreateChannelResult> {
        let session_id = new_session_id();
        let channel_id = new_channel_id();
        let channel_type = require_nonempty_trimmed(channel_type, "channel_type")?.to_lowercase();

        self.init_session(&session_id, name)?;

        let entry = ChannelEntry {
            channel_id: channel_id.clone(),
            session_id: session_id.clone(),
            channel_type,
            data,
        };
        self.channel_catalog.add(entry.clone())?;

        self.channels
            .start(entry)
            .map_err(|e| InvalidContent::new(e))?;

        Ok(CreateChannelResult {
            channel_id,
            session_id,
        })
    }

    pub fn update_channel(&self, channel_id: &str, data: Value) -> Result<()> {
        let entry = self.channel_catalog.get(channel_id)?;
        self.channel_catalog
            .upsert(channel_id, &entry.channel_type, data)?;
        let entry = self.channel_catalog.get(channel_id)?;
        self.channels
            .restart(entry)
            .map_err(|e| InvalidContent::new(e))?;
        Ok(())
    }

    pub fn delete_channel(&self, channel_id: &str) -> Result<()> {
        let channel_id = require_nonempty_trimmed(channel_id, "channel_id")?;
        self.channels.stop(&channel_id);
        let entry = self.channel_catalog.remove(&channel_id)?;
        if let Err(e) = self.delete_session(&entry.session_id) {
            if !matches!(e, SondaError::MissingReference(_)) {
                return Err(e);
            }
            tracing::warn!(
                session_id = %entry.session_id,
                channel_id = %channel_id,
                "delete_channel: session already removed; channel catalog cleaned up"
            );
        }
        Ok(())
    }

    fn init_session(&self, session_id: &str, name: &str) -> Result<()> {
        let display_name = self.session_catalog
            .add_session_entry(session_id, name)?;

        self.session_transcripts
            .create(session_id)
            .map_err(SondaError::from)?;

        self.snapshot
            .notify_session_added(session_id, &display_name);

        Ok(())
    }

    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        let session_id = require_nonempty_trimmed(session_id, "session_id")?;

        self.live_sessions.evict(&session_id);

        if self.session_transcripts.exists(&session_id) {
            self.session_transcripts
                .remove(&session_id)
                .map_err(SondaError::from)?;
        }

        self.session_catalog.remove_session_entry(&session_id)?;

        self.snapshot.notify_session_removed(&session_id);

        Ok(())
    }

    pub fn set_session_agent_id(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> Result<()> {
        let session_id = require_nonempty_trimmed(session_id, "session_id")?;
        let agent_id = require_nonempty_trimmed(agent_id, "agent_id")?;
        ensure_known_agent(&self.settings_store, &agent_id)?;

        self.session_catalog
            .set_session_agent_id(&session_id, &agent_id)?;

        Ok(())
    }

    /// Starts in-process background services (IM connectors, …).
    ///
    /// Application hosts should call this after [`SondaBuilder::build`] / bootstrap, not individual
    /// service starters, so new services do not require host-specific wiring.
    pub async fn startup(&self) {
        self.start_channels().await;
    }

    /// Stops background services started by [`Self::startup`], in reverse order.
    pub async fn shutdown(&self) {
        self.stop_channels().await;
    }

    /// Starts IM connectors for every entry in [`Self::channel_catalog`].
    pub async fn start_channels(&self) {
        self.channels.start_all(self.channel_catalog.entries());
    }

    /// Stops IM connectors and waits for listener tasks to exit (bounded timeout per channel).
    pub async fn stop_channels(&self) {
        self.channels.shutdown_all().await;
    }
}

pub struct SondaBuilder {
    settings_store: Option<Arc<SondaSettingsStore>>,
    skill_center: Option<SkillCenter>,
    skill_hub: Option<SkillHub>,
    session_catalog: Option<Arc<SondaSessionCatalog>>,
    session_transcripts: Option<Arc<SondaSessionTranscripts>>,
    channel_catalog: Option<Arc<ChannelCatalog>>,
    session_workspace: Option<Arc<SondaSessionWorkspace>>,
    tool_catalog: Option<SondaToolCatalog>,
    tool_regs: Option<Vec<SondaToolRegistration>>,
    channel_factories: Option<HashMap<String, ChannelFactoryFn>>,
}

impl Default for SondaBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SondaBuilder {
    pub fn new() -> Self {
        Self {
            settings_store: None,
            skill_center: None,
            skill_hub: None,
            session_catalog: None,
            session_transcripts: None,
            channel_catalog: None,
            session_workspace: None,
            tool_catalog: None,
            tool_regs: None,
            channel_factories: None,
        }
    }

    pub fn settings(mut self, settings_store: Arc<SondaSettingsStore>) -> Self {
        self.settings_store = Some(settings_store);
        self
    }

    pub fn skill_center(mut self, skill_center: SkillCenter) -> Self {
        self.skill_center = Some(skill_center);
        self
    }

    pub fn skill_hub(mut self, skill_hub: SkillHub) -> Self {
        self.skill_hub = Some(skill_hub);
        self
    }

    pub fn session_catalog(mut self, session_catalog: Arc<SondaSessionCatalog>) -> Self {
        self.session_catalog = Some(session_catalog);
        self
    }

    pub fn session_transcripts(mut self, session_transcripts: Arc<SondaSessionTranscripts>) -> Self {
        self.session_transcripts = Some(session_transcripts);
        self
    }

    pub fn channel_catalog(mut self, channel_catalog: Arc<ChannelCatalog>) -> Self {
        self.channel_catalog = Some(channel_catalog);
        self
    }

    /// Inputs for [`SondaAgentRunner`] (workspace must be created by the host before [`Self::build`]).
    pub fn harness_components(
        mut self,
        session_workspace: Arc<SondaSessionWorkspace>,
        tool_catalog: SondaToolCatalog,
        tools: Vec<SondaToolRegistration>,
    ) -> Self {
        self.session_workspace = Some(session_workspace);
        self.tool_catalog = Some(tool_catalog);
        self.tool_regs = Some(tools);
        self
    }

    pub fn channel_factories(
        mut self,
        channel_factories: HashMap<String, ChannelFactoryFn>,
    ) -> Self {
        self.channel_factories = Some(channel_factories);
        self
    }

    pub fn build(self) -> Result<Sonda> {
        let settings_store = self
            .settings_store
            .ok_or_else(|| error_missing_field("settings_store"))?;

        let skill_center = self
            .skill_center
            .ok_or_else(|| error_missing_field("skill_center"))?;

        let skill_hub = self
            .skill_hub
            .ok_or_else(|| error_missing_field("skill_hub"))?;

        let session_catalog = self
            .session_catalog
            .ok_or_else(|| error_missing_field("session_catalog"))?;

        let session_transcripts = self
            .session_transcripts
            .ok_or_else(|| error_missing_field("session_transcripts"))?;

        let channel_catalog = self
            .channel_catalog
            .ok_or_else(|| error_missing_field("channel_catalog"))?;

        let session_workspace = self
            .session_workspace
            .ok_or_else(|| error_missing_field("session_workspace"))?;

        let harness_tool_catalog = self
            .tool_catalog
            .ok_or_else(|| error_missing_field("session_harness"))?;

        let harness_tools = self
            .tool_regs
            .ok_or_else(|| error_missing_field("session_harness"))?;

        let channel_factories = self
            .channel_factories
            .ok_or_else(|| error_missing_field("channel_factories"))?;

        let authorizer = create_authorizer(settings_store.as_ref());

        let agent_runner = Arc::new(SondaAgentRunner::new(
            settings_store.clone(),
            session_catalog.clone(),
            authorizer.clone(),
            harness_tool_catalog,
            session_workspace.clone(),
            harness_tools,
            true,
        )?);

        validate_dependencies(
            settings_store.as_ref(),
            session_catalog.as_ref(),
            agent_runner.as_ref(),
        )?;

        let snapshot = Arc::new(SondaSnapshot::new(session_catalog.as_ref()));
        session_transcripts.set_hook(snapshot.clone());

        let session_factory = Arc::new(SondaSessionFactory::new(
            settings_store.clone(),
            skill_center.clone(),
            session_catalog.clone(),
            session_transcripts.clone(),
            agent_runner.clone(),
        ));

        let live_sessions = LiveSessions::new(session_factory);

        let channels = Arc::new(ChannelsManager::new(
            session_transcripts.clone(),
            authorizer.clone(),
            live_sessions.clone(),
            channel_factories,
        ));

        Ok(Sonda::new(
            settings_store,
            skill_center,
            session_catalog,
            session_transcripts,
            session_workspace,
            agent_runner,
            skill_hub,
            authorizer,
            snapshot,
            live_sessions,
            channel_catalog,
            channels,
        ))
    }
}

fn validate_dependencies(
    settings: &SondaSettingsStore,
    sessions: &SondaSessionCatalog,
    agent_runner: &SondaAgentRunner,
) -> Result<()> {
    let settings_catalog = settings.catalog();

    for agent in &settings_catalog.agents {
        agent_runner.validate_allowed_tools(&agent.allowed_tools)?;
    }

    let default_agent_id = sessions.default_agent_id();

    if !settings.has_agent(&default_agent_id) {
        return Err(InvalidContent::new(format!(
            "`default_agent_id` (`{default_agent_id}`) does not reference an agent defined in server settings",
        ))
        .into());
    }

    for entry in sessions.entries() {
        let Some(aid) = entry.agent_id.as_deref() else {
            continue;
        };

        if !settings.has_agent(aid) {
            return Err(MissingReference::new(format!(
                "entries row session_id `{}` references unknown agent `{aid}`",
                entry.session_id,
            ))
            .into());
        }
    }

    Ok(())
}

fn create_authorizer(_settings: &SondaSettingsStore) -> Arc<dyn ToolCallAuthorizer> {
    // TODO: select implementation from settings when auth profiles land in TOML.
    Arc::new(AlwaysAsking::new())
}

fn ensure_known_agent(settings_store: &SondaSettingsStore, agent_id: &str) -> Result<()> {
    if !settings_store.has_agent(agent_id) {
        return Err(MissingReference::new(format!("unknown agent id `{agent_id}`")).into());
    }
    Ok(())
}

fn error_missing_field(name: &'static str) -> crate::error::SondaError {
    InvalidArguments::new(name, "missing from SondaBuilder").into()
}

fn new_session_id() -> String {
    new_id()
}

fn new_channel_id() -> String {
    new_id()
}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before UNIX_EPOCH")
        .as_nanos();
    format!("{nanos:x}")
}
