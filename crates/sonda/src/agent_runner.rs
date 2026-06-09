//! Shared [`SondaAgentRunner`] for Sonda live chat sessions.

use std::collections::HashSet;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use moray_extensions::completions::{Endpoint, OpenAIChatCompletion};
use moray_core::{
    AgentRequestBuilder, AgentResponseEvent, ChatCompletion, ContextEngine, Tool,
    ToolCallAuthorizer, ToolboxBuilder,
};
use moray_session::{AgentRunner, SessionError};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::{InvalidContent, Result, SondaError};
use crate::{
    SondaSessionCatalog, SondaSessionWorkspace, SondaSettingsStore, SondaToolCatalog,
};

/// Public catalog row for settings UI (`GET /settings/catalog`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SondaToolCatalogEntry {
    pub id: String,
    pub label: String,
}

/// One registered tool id and its per-session instance builder.
///
/// [`SondaAgentRunner::create_toolbox`] passes per-session dir from [`SondaSessionWorkspace`].
/// Tools that do not use a workspace may ignore it (e.g. `|_|`).
pub struct SondaToolRegistration {
    pub name: &'static str,
    pub build: Box<dyn Fn(PathBuf) -> Arc<dyn Tool> + Send + Sync>,
}

impl SondaToolRegistration {
    pub fn new(
        name: &'static str,
        build: impl Fn(PathBuf) -> Arc<dyn Tool> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name,
            build: Box::new(build),
        }
    }
}

/// Shared agent runner: toolbox, TOML-backed completion, and agent stream assembly.
///
/// Holds snapshots of shared [`SondaSettingsStore`] / [`SondaSessionCatalog`] so completions
/// pick up dynamic agent changes without restarting the session.
pub struct SondaAgentRunner {
    settings_store: Arc<SondaSettingsStore>,
    session_catalog: Arc<SondaSessionCatalog>,
    authorizer: Arc<dyn ToolCallAuthorizer>,
    catalog: SondaToolCatalog,
    workspace: Arc<SondaSessionWorkspace>,
    registrations: Vec<SondaToolRegistration>,
    stream: bool,
}

impl SondaAgentRunner {
    pub fn new(
        settings_store: Arc<SondaSettingsStore>,
        session_catalog: Arc<SondaSessionCatalog>,
        authorizer: Arc<dyn ToolCallAuthorizer>,
        catalog: SondaToolCatalog,
        workspace: Arc<SondaSessionWorkspace>,
        registrations: Vec<SondaToolRegistration>,
        stream: bool,
    ) -> Result<Self> {
        if registrations.is_empty() {
            return Err(InvalidContent::new("at least one tool must be registered").into());
        }

        let mut seen = HashSet::new();
        for reg in &registrations {
            let name = reg.name;
            if name.trim().is_empty() {
                return Err(InvalidContent::new("registered tool id must not be empty").into());
            }
            if !catalog.is_known(name) {
                return Err(InvalidContent::new(format!(
                    "tool `{name}` is not defined in the tool catalog"
                ))
                .into());
            }
            if !seen.insert(name) {
                return Err(InvalidContent::new(format!("duplicate tool id `{name}`")).into());
            }
        }

        let allow: Vec<&str> = registrations.iter().map(|r| r.name).collect();
        let catalog = catalog.filter(&allow);

        Ok(Self {
            settings_store,
            session_catalog,
            authorizer,
            catalog,
            workspace,
            registrations,
            stream,
        })
    }

    /// Registered tools for settings UI (`GET /tools`), in registration order.
    pub fn tool_entries(&self) -> Vec<SondaToolCatalogEntry> {
        self.registrations
            .iter()
            .map(|reg| SondaToolCatalogEntry {
                id: reg.name.to_string(),
                label: self.catalog.label(reg.name).to_string(),
            })
            .collect()
    }

    pub fn is_known(&self, name: &str) -> bool {
        self.registrations.iter().any(|reg| reg.name == name)
    }

    pub fn authorizer(&self) -> Arc<dyn ToolCallAuthorizer> {
        self.authorizer.clone()
    }

    pub fn validate_allowed_tools(&self, allowed_tools: &[String]) -> Result<()> {
        if allowed_tools.is_empty() {
            return Ok(());
        }

        let mut seen = HashSet::new();
        for name in allowed_tools {
            if name.trim().is_empty() {
                return Err(InvalidContent::new("agents.allowed_tools entry is empty").into());
            }
            if !self.is_known(name) {
                return Err(InvalidContent::new(format!(
                    "allowed_tools contains unknown tool `{name}`"
                ))
                .into());
            }
            if !seen.insert(name.as_str()) {
                return Err(InvalidContent::new(format!(
                    "allowed_tools contains duplicate `{name}`"
                ))
                .into());
            }
        }
        Ok(())
    }

    fn resolve_session_agent_id(&self, session_id: &str) -> Result<String> {
        Ok(self
            .session_catalog
            .get_session_agent_id(session_id)?)
    }

    fn resolve_session_completion_endpoint(&self, session_id: &str) -> Result<Endpoint> {
        let agent_id = self.resolve_session_agent_id(session_id)?;
        self.settings_store
            .resolve_completion_endpoint(&agent_id)
    }

    fn create_toolbox(&self, session_id: &str) -> std::result::Result<Arc<moray_core::Toolbox>, SessionError> {
        let agent_id = self
            .resolve_session_agent_id(session_id)
            .map_err(|e| SessionError::from(moray_core::MorayError::from(e)))?;
        let allowed_tools = self
            .settings_store
            .agent_allowed_tools(&agent_id)
            .map_err(|e| SessionError::from(moray_core::MorayError::from(e)))?;

        let allow: HashSet<&str> = allowed_tools.iter().map(String::as_str).collect();
        let session_dir = self.workspace.session_dir(session_id);

        let mut manifests = Vec::new();
        let mut tools = Vec::new();
        for reg in &self.registrations {
            if allowed_tools.is_empty() || !allow.contains(reg.name) {
                continue;
            }
            let tool = (reg.build)(session_dir.clone());
            if tool.name() != reg.name {
                let err: SondaError = InvalidContent::new(format!(
                    "tool builder for `{}` returned mismatched tool `{}`",
                    reg.name,
                    tool.name()
                ))
                .into();
                return Err(SessionError::from(moray_core::MorayError::from(err)));
            }
            if let Some(manifest) = self.catalog.manifest(reg.name).cloned() {
                manifests.push(manifest);
                tools.push(tool);
            }
        }

        let mut builder = ToolboxBuilder::new().manifests(manifests);
        for tool in tools {
            builder = builder.tool(tool);
        }
        Ok(Arc::new(builder.auth(self.authorizer.clone()).build()))
    }

    fn create_completion(
        &self,
        session_id: &str,
    ) -> std::result::Result<Arc<dyn ChatCompletion>, SessionError> {
        let endpoint = self
            .resolve_session_completion_endpoint(session_id)
            .map_err(|e| SessionError::from(moray_core::MorayError::from(e)))?;
        Ok(Arc::new(OpenAIChatCompletion::new(endpoint)))
    }
}

impl AgentRunner for SondaAgentRunner {
    fn create_agent_stream(
        &self,
        session_id: &str,
        context: Arc<dyn ContextEngine>,
        cancellation: CancellationToken,
    ) -> std::result::Result<Pin<Box<dyn Stream<Item = AgentResponseEvent> + Send>>, SessionError>
    {
        let completion = self.create_completion(session_id)?;
        let toolbox = self.create_toolbox(session_id)?;

        AgentRequestBuilder::new()
            .completion(completion)
            .toolbox(toolbox)
            .context(context)
            .stream(self.stream)
            .cancellation(cancellation)
            .run()
            .map_err(SessionError::from)
    }
}
