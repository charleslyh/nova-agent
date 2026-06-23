mod harness;
mod request;
mod sink;
mod trigger;

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::{ContextEngine, MorayError};

pub use harness::AgentHarnessFactory;
pub use request::MultiAgentsRequestBuilder;
pub use sink::{
    AgentRole, ChannelMultiAgentEventSink, MultiAgentEventSink, MultiAgentResponseEvent,
};
pub use trigger::{SubAgentContextMode, SubAgentSpec, RUN_SUB_AGENT_TOOL_NAME};

#[async_trait]
pub trait AgentRunner: Send + Sync {
    async fn run(
        &self,
        context: Arc<dyn ContextEngine>,
        cancellation: CancellationToken,
        events: Arc<dyn MultiAgentEventSink>,
    ) -> std::result::Result<(), MorayError>;
}
