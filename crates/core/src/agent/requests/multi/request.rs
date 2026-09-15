use std::sync::Arc;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::agent::requests::multi::harness::AgentHarnessFactory;
use crate::agent::requests::multi::sink::{bridge_agent_events, AgentRole, MultiAgentEventSink};
use crate::agent::requests::multi::trigger::{inject_sub_agents_trigger, SubAgentSpec};
use crate::agent::requests::single::AgentRequestBuilder;
use crate::{ContextEngine, NovaError, Toolbox};

/// Builds arguments for a multi-agent leader run. [`Self::factory`], [`Self::leader_agent_id`],
/// and [`Self::context`] are required.
pub struct MultiAgentsRequestBuilder {
    factory: Option<Arc<dyn AgentHarnessFactory>>,
    leader_agent_id: Option<String>,
    context: Option<Arc<dyn ContextEngine>>,
    sub_agents: Vec<SubAgentSpec>,
    stream: bool,
    leader_max_rounds: Option<usize>,
    cancellation: Option<CancellationToken>,
}

impl Default for MultiAgentsRequestBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiAgentsRequestBuilder {
    pub fn new() -> Self {
        Self {
            factory: None,
            leader_agent_id: None,
            context: None,
            sub_agents: Vec::new(),
            stream: true,
            leader_max_rounds: None,
            cancellation: None,
        }
    }

    pub fn factory(mut self, factory: Arc<dyn AgentHarnessFactory>) -> Self {
        self.factory = Some(factory);
        self
    }

    pub fn leader_agent_id(mut self, leader_agent_id: impl Into<String>) -> Self {
        self.leader_agent_id = Some(leader_agent_id.into());
        self
    }

    pub fn context(mut self, context: Arc<dyn ContextEngine>) -> Self {
        self.context = Some(context);
        self
    }

    pub fn sub_agents(mut self, sub_agents: Vec<SubAgentSpec>) -> Self {
        self.sub_agents = sub_agents;
        self
    }

    pub fn stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    pub fn leader_max_rounds(mut self, max_rounds: usize) -> Self {
        self.leader_max_rounds = Some(max_rounds);
        self
    }

    pub fn cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub fn run(
        self,
        events: Arc<dyn MultiAgentEventSink>,
    ) -> std::result::Result<JoinHandle<std::result::Result<(), NovaError>>, NovaError> {
        let Some(factory) = self.factory else {
            return Err(NovaError::Message(
                "MultiAgentsRequestBuilder: missing required `factory`".into(),
            ));
        };
        let Some(leader_agent_id) = self.leader_agent_id else {
            return Err(NovaError::Message(
                "MultiAgentsRequestBuilder: missing required `leader_agent_id`".into(),
            ));
        };
        let Some(context) = self.context else {
            return Err(NovaError::Message(
                "MultiAgentsRequestBuilder: missing required `context`".into(),
            ));
        };

        let cancellation = self.cancellation.unwrap_or_default();

        let leader_sink =
            bridge_agent_events(events.clone(), leader_agent_id.clone(), AgentRole::Leader);

        let toolbox = create_leader_toolbox(
            factory.clone(),
            self.stream,
            leader_agent_id.as_str(),
            self.sub_agents.as_slice(),
            context.clone(),
            events,
            cancellation.clone(),
        )?;

        let completion = factory.create_completion(leader_agent_id.as_str())?;

        let mut request = AgentRequestBuilder::new()
            .completion(completion)
            .toolbox(toolbox)
            .context(context)
            .stream(self.stream);
        if let Some(max_rounds) = self.leader_max_rounds {
            request = request.max_rounds(max_rounds);
        }

        request.cancellation(cancellation).run(leader_sink)
    }
}

fn create_leader_toolbox(
    factory: Arc<dyn AgentHarnessFactory>,
    stream: bool,
    leader_agent_id: &str,
    sub_agents: &[SubAgentSpec],
    leader_context: Arc<dyn ContextEngine>,
    events: Arc<dyn MultiAgentEventSink>,
    cancellation: CancellationToken,
) -> std::result::Result<Arc<Toolbox>, NovaError> {
    let mut toolbox = factory.create_toolbox(leader_agent_id)?;

    inject_sub_agents_trigger(
        &mut toolbox,
        sub_agents,
        factory,
        stream,
        leader_context,
        events,
        cancellation,
    );

    Ok(Arc::new(toolbox))
}
