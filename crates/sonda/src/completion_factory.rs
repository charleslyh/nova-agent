//! Per-agent [`ChatCompletion`] construction from settings and registered providers.

use std::collections::HashSet;
use std::sync::Arc;

use moray_core::ChatCompletion;
use tracing::warn;

use crate::error::{InvalidContent, Result};
use crate::settings_store::SondaSettingsCompletionEntry;
use crate::SondaSettingsStore;

/// One registered completion provider and its builder.
pub struct SondaCompletionRegistration {
    pub provider: &'static str,
    build: Box<dyn Fn(SondaSettingsCompletionEntry) -> Result<Arc<dyn ChatCompletion>> + Send + Sync>,
}

impl SondaCompletionRegistration {
    pub fn new(
        provider: &'static str,
        build: impl Fn(SondaSettingsCompletionEntry) -> Result<Arc<dyn ChatCompletion>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            provider,
            build: Box::new(build),
        }
    }
}

/// Resolves agent completion settings and delegates construction to a registered provider.
pub struct SondaCompletionFactory {
    settings_store: Arc<SondaSettingsStore>,
    registrations: Vec<SondaCompletionRegistration>,
}

impl SondaCompletionFactory {
    pub fn new(
        settings_store: Arc<SondaSettingsStore>,
        registrations: Vec<SondaCompletionRegistration>,
    ) -> Result<Self> {
        if registrations.is_empty() {
            return Err(
                InvalidContent::new("at least one completion provider must be registered").into(),
            );
        }

        let mut seen = HashSet::new();
        for reg in &registrations {
            if reg.provider.trim().is_empty() {
                return Err(InvalidContent::new("completion provider must not be empty").into());
            }
            if !seen.insert(reg.provider) {
                return Err(InvalidContent::new(format!(
                    "duplicate completion provider `{}`",
                    reg.provider
                ))
                .into());
            }
        }

        Ok(Self {
            settings_store,
            registrations,
        })
    }

    pub fn create_completion(&self, agent_id: &str) -> Result<Arc<dyn ChatCompletion>> {
        let entry = self
            .settings_store
            .completion_entry_for_agent(agent_id)?;

        let provider = entry.provider.trim();
        if provider.is_empty() {
            warn!(agent_id, "completion provider is empty; skipping");
            return Err(InvalidContent::new("completion provider is empty").into());
        }

        let Some(reg) = self
            .registrations
            .iter()
            .find(|r| r.provider == provider)
        else {
            warn!(
                agent_id,
                provider,
                "unknown completion provider; skipping"
            );

            return Err(InvalidContent::new(format!(
                "unknown completion provider `{provider}`"
            ))
            .into());
        };

        (reg.build)(entry)
    }
}
