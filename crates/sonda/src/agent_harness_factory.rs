//! [`AgentHarnessFactory`] for one live sonda session.

use std::sync::Arc;

use moray_core::{
    AgentHarnessFactory, AgentResultFormatter, ChatCompletion, ChatCompletionRequestMessage,
    ContextEngine, MorayError, Toolbox,
};

use crate::completion_factory::SondaCompletionFactory;
use crate::context::{ContextBuilder, SondaContextFactory};
use crate::toolbox_factory::{SessionToolboxFactory, SondaToolboxFactory};

pub struct SondaAgentHarnessFactory {
    completion_factory: Arc<SondaCompletionFactory>,
    toolbox_factory: SessionToolboxFactory,
    context_factory: SondaContextFactory,
}

impl SondaAgentHarnessFactory {
    pub fn for_session(
        session_id: impl Into<String>,
        completion_factory: Arc<SondaCompletionFactory>,
        toolbox_factory: Arc<SondaToolboxFactory>,
        context_builder: ContextBuilder,
    ) -> Arc<Self> {
        Arc::new(Self {
            completion_factory,
            toolbox_factory: SessionToolboxFactory::new(session_id, toolbox_factory),
            context_factory: SondaContextFactory::new(context_builder),
        })
    }
}

impl AgentHarnessFactory for SondaAgentHarnessFactory {
    fn create_completion(
        &self,
        agent_id: &str,
    ) -> std::result::Result<Arc<dyn ChatCompletion>, MorayError> {
        self.completion_factory
            .create_completion(agent_id)
            .map_err(MorayError::from)
    }

    fn create_toolbox(&self, agent_id: &str) -> std::result::Result<Toolbox, MorayError> {
        self.toolbox_factory.create_toolbox(agent_id)
    }

    fn create_context(
        &self,
        agent_id: &str,
        messages: Vec<ChatCompletionRequestMessage>,
    ) -> std::result::Result<Arc<dyn ContextEngine>, MorayError> {
        self.context_factory.create_context(agent_id, messages)
    }

    fn create_result_formatter(
        &self,
        _agent_id: &str,
    ) -> Option<Arc<dyn AgentResultFormatter>> {
        None
    }
}
