use std::sync::Arc;

use crate::{
    ChatCompletion, ChatCompletionRequestMessage, ContextEngine, MorayError, Toolbox,
};

use super::formatter::AgentResultFormatter;

/// Host-provided construction of completion, toolbox, and context for one agent run scope.
pub trait AgentHarnessFactory: Send + Sync {
    fn create_completion(
        &self,
        agent_id: &str,
    ) -> std::result::Result<Arc<dyn ChatCompletion>, MorayError>;

    fn create_toolbox(&self, agent_id: &str) -> std::result::Result<Toolbox, MorayError>;

    fn create_context(
        &self,
        agent_id: &str,
        messages: Vec<ChatCompletionRequestMessage>,
    ) -> std::result::Result<Arc<dyn ContextEngine>, MorayError>;

    /// Optionally provide a result formatter for a sub-agent.
    ///
    /// When present, the formatter transforms the sub-agent's intermediate run data
    /// (tool call records, etc.) into a structured result string, bypassing the default
    /// "last text block" behavior.
    ///
    /// Default implementation returns `None` (no custom formatting).
    fn create_result_formatter(
        &self,
        _agent_id: &str,
    ) -> Option<Arc<dyn AgentResultFormatter>> {
        None
    }
}
