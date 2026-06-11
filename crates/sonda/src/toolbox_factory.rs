//! Harness tool catalog, registration, and per-agent [`Toolbox`] construction.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use moray_core::{Tool, ToolCallAuthorizer, Toolbox, ToolboxBuilder};
use serde::{Deserialize, Serialize};

use crate::error::{InvalidContent, Result};
use crate::{SondaSessionWorkspace, SondaSettingsStore, SondaToolCatalog};

/// Public catalog row for settings UI (`GET /tools`).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SondaToolCatalogEntry {
    pub id: String,
    pub label: String,
}

/// One registered tool id and its per-session instance builder.
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

/// Per-agent [`Toolbox`] construction backed by registered built-in tools and session settings.
pub struct SondaToolboxFactory {
    settings_store: Arc<SondaSettingsStore>,
    authorizer: Arc<dyn ToolCallAuthorizer>,
    catalog: SondaToolCatalog,
    registrations: Vec<SondaToolRegistration>,
    workspace: Arc<SondaSessionWorkspace>,
}

impl SondaToolboxFactory {
    pub fn new(
        settings_store: Arc<SondaSettingsStore>,
        authorizer: Arc<dyn ToolCallAuthorizer>,
        catalog: SondaToolCatalog,
        registrations: Vec<SondaToolRegistration>,
        workspace: Arc<SondaSessionWorkspace>,
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
            authorizer,
            catalog,
            registrations,
            workspace,
        })
    }

    pub fn tools(&self) -> Vec<SondaToolCatalogEntry> {
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

    pub fn create_toolbox(&self, session_id: &str, agent_id: &str) -> Result<Toolbox> {
        let allowed_tools = self.settings_store.agent_allowed_tools(agent_id)?;

        let allow: HashSet<&str> = allowed_tools.iter().map(String::as_str).collect();
        let tool_root = self.workspace.session_output_dir(session_id);

        let mut manifests = Vec::new();
        let mut tools = Vec::new();
        for reg in &self.registrations {
            if allowed_tools.is_empty() || !allow.contains(reg.name) {
                continue;
            }
            let tool = (reg.build)(tool_root.clone());
            if tool.name() != reg.name {
                return Err(InvalidContent::new(format!(
                    "tool builder for `{}` returned mismatched tool `{}`",
                    reg.name,
                    tool.name()
                ))
                .into());
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
        Ok(builder.auth(self.authorizer.clone()).build())
    }
}
