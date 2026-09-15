//! System preamble generation for [`super::CompositeContextEngine`].

use nova_core::{ChatCompletionRequestMessage, NovaError, ToolManifest};

/// Builds system prompt text for one agent run.
///
/// The engine calls [`Self::generate`] once per [`nova_core::ContextEngine::setup`],
/// stores the result as the run-scoped preamble, and prepends it on each
/// [`nova_core::ContextEngine::assemble`]. Providers stay stateless across runs.
pub trait PreambleProvider: Send + Sync {
    fn generate(
        &self,
        transcript: &[ChatCompletionRequestMessage],
        tools: &[ToolManifest],
    ) -> Result<String, NovaError>;
}
