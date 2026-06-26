//! Sonda [`AgentRunner`] and [`SondaAgentRunnerFactory`].

use std::sync::Arc;

use async_trait::async_trait;
use moray_core::{MorayError, MultiAgentsRequestBuilder};
use moray_session::AgentRunner;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::agent_harness_factory::SondaAgentHarnessFactory;
use crate::completion_factory::SondaCompletionFactory;
use crate::context::ContextBuilder;
use crate::error::SondaError;
use crate::toolbox_factory::SondaToolboxFactory;
use crate::SondaSessionCatalog;

pub use moray_core::RUN_SUB_AGENT_TOOL_NAME;

/// Creates session-bound [`SondaAgentRunner`] values.
pub struct SondaAgentRunnerFactory {
    session_catalog: Arc<SondaSessionCatalog>,
    completion_factory: Arc<SondaCompletionFactory>,
    toolbox_factory: Arc<SondaToolboxFactory>,
    context_builder: ContextBuilder,
    stream: bool,
}

impl SondaAgentRunnerFactory {
    pub fn new(
        completion_factory: Arc<SondaCompletionFactory>,
        toolbox_factory: Arc<SondaToolboxFactory>,
        session_catalog: Arc<SondaSessionCatalog>,
        context_builder: ContextBuilder,
        stream: bool,
    ) -> Self {
        Self {
            session_catalog,
            completion_factory,
            toolbox_factory,
            context_builder,
            stream,
        }
    }

    pub fn create(&self, session_id: impl Into<String>) -> Arc<SondaAgentRunner> {
        Arc::new(SondaAgentRunner::new(session_id, self))
    }
}

/// Session-bound [`AgentRunner`] for one live chat session.
pub struct SondaAgentRunner {
    session_id: String,
    session_catalog: Arc<SondaSessionCatalog>,
    harness_factory: Arc<SondaAgentHarnessFactory>,
    stream: bool,
}

impl SondaAgentRunner {
    fn new(session_id: impl Into<String>, factory: &SondaAgentRunnerFactory) -> Self {
        let session_id = session_id.into();
        let harness_factory = SondaAgentHarnessFactory::for_session(
            session_id.clone(),
            factory.completion_factory.clone(),
            factory.toolbox_factory.clone(),
            factory.context_builder.clone(),
        );

        Self {
            session_id,
            session_catalog: factory.session_catalog.clone(),
            harness_factory,
            stream: factory.stream,
        }
    }
}

#[async_trait]
impl AgentRunner for SondaAgentRunner {
    async fn run(
        &self,
        context: Arc<dyn moray_core::ContextEngine>,
        cancellation: CancellationToken,
        events: Arc<dyn moray_core::MultiAgentEventSink>,
    ) -> std::result::Result<(), MorayError> {
        let leader_agent_id = self
            .session_catalog
            .get_session_agent_id(self.session_id.as_str())
            .map_err(|e| MorayError::from(SondaError::from(e)))?;

        let sub_agents = self
            .session_catalog
            .get_session_sub_agents(self.session_id.as_str())
            .map_err(|e| MorayError::from(SondaError::from(e)))?;

        info!(
            session_id = %self.session_id,
            sub_agent_count = sub_agents.len(),
            "agent run started"
        );

        match MultiAgentsRequestBuilder::new()
            .factory(self.harness_factory.clone())
            .stream(self.stream)
            .leader_agent_id(leader_agent_id)
            .sub_agents(sub_agents)
            .context(context)
            .cancellation(cancellation)
            .run(events)?
            .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => {
                warn!(
                    session_id = %self.session_id,
                    error = %err,
                    "agent run failed"
                );
                Err(err)
            }
            Err(join_err) => {
                warn!(
                    session_id = %self.session_id,
                    error = %join_err,
                    "agent run task join failed"
                );
                Err(MorayError::Message(format!(
                    "agent run task failed: {join_err}"
                )))
            }
        }
    }
}
