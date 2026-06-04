//! System preamble generation for [`super::CompositeContextEngine`].

use moray_core::{ChatCompletionRequestMessage, MorayError, ToolManifest};

/// Builds system prompt text for one agent run.
///
/// The engine calls [`Self::generate`] once per [`moray_core::ContextEngine::setup`],
/// stores the result as the run-scoped preamble, and prepends it on each
/// [`moray_core::ContextEngine::assemble`]. Providers stay stateless across runs.
pub trait PreambleProvider: Send + Sync {
    fn generate(
        &self,
        transcript: &[ChatCompletionRequestMessage],
        tools: &[ToolManifest],
    ) -> Result<String, MorayError>;
}
