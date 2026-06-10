//! Per-agent [`ChatCompletion`] construction from settings and a registered backend.

use std::sync::Arc;

use moray_core::ChatCompletion;
use moray_extensions::completions::Endpoint;

use crate::error::Result;
use crate::SondaSettingsStore;

/// Builds a [`ChatCompletion`] from a resolved settings [`Endpoint`].
pub struct SondaCompletionRegistration {
    build: Box<dyn Fn(Endpoint) -> Arc<dyn ChatCompletion> + Send + Sync>,
}

impl SondaCompletionRegistration {
    pub fn new(
        build: impl Fn(Endpoint) -> Arc<dyn ChatCompletion> + Send + Sync + 'static,
    ) -> Self {
        Self {
            build: Box::new(build),
        }
    }
}

/// Resolves agent completion settings and delegates construction to a registered backend.
pub struct SondaCompletionFactory {
    settings_store: Arc<SondaSettingsStore>,
    registration: SondaCompletionRegistration,
}

impl SondaCompletionFactory {
    pub fn new(
        settings_store: Arc<SondaSettingsStore>,
        registration: SondaCompletionRegistration,
    ) -> Self {
        Self {
            settings_store,
            registration,
        }
    }

    pub fn create_completion(&self, agent_id: &str) -> Result<Arc<dyn ChatCompletion>> {
        let endpoint = self.settings_store.resolve_completion_endpoint(agent_id)?;
        Ok((self.registration.build)(endpoint))
    }
}
