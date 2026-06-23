//! Injected [`ContextEngine`] construction for live sessions.

use std::sync::Arc;

use moray_core::{ChatCompletionRequestMessage, ContextEngine, MorayError};

use crate::error::Result;

pub type ContextBuilder = Arc<
    dyn Fn(&str, Vec<ChatCompletionRequestMessage>) -> Result<Arc<dyn ContextEngine>>
        + Send
        + Sync,
>;

pub(crate) struct SondaContextFactory {
    inner: ContextBuilder,
}

impl SondaContextFactory {
    pub(crate) fn new(inner: ContextBuilder) -> Self {
        Self { inner }
    }

    pub(crate) fn create_context(
        &self,
        agent_id: &str,
        messages: Vec<ChatCompletionRequestMessage>,
    ) -> std::result::Result<Arc<dyn ContextEngine>, MorayError> {
        (self.inner)(agent_id, messages).map_err(MorayError::from)
    }
}
