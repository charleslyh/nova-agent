use async_trait::async_trait;

use crate::agent::types::AgentResponseEvent;

/// Push sink for single-agent [`AgentResponseEvent`] values.
/// Emit failures are ignored, matching channel `send` best-effort semantics.
#[async_trait]
pub trait AgentEventSink: Send + Sync {
    async fn emit(&self, event: AgentResponseEvent);
}
