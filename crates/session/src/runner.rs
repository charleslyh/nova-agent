use std::sync::Arc;

use async_trait::async_trait;
use moray_core::{ContextEngine, MorayError, MultiAgentEventSink};
use tokio_util::sync::CancellationToken;

/// Host-provided executor for one agent turn inside [`crate::SessionRuntime`].
#[async_trait]
pub trait AgentRunner: Send + Sync {
    async fn run(
        &self,
        context: Arc<dyn ContextEngine>,
        cancellation: CancellationToken,
        events: Arc<dyn MultiAgentEventSink>,
    ) -> std::result::Result<(), MorayError>;
}
