use std::sync::Arc;

use crate::{
    ChatCompletion, ChatCompletionRequestMessage, ContextEngine, MorayError, Toolbox,
};

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
}
