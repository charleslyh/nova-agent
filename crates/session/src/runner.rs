use std::pin::Pin;
use std::sync::Arc;

use futures::Stream;
use tokio_util::sync::CancellationToken;

use moray_core::{AgentResponseEvent, ContextEngine};

use crate::Result;

pub trait AgentRunner: Send + Sync {
    fn create_agent_stream(
        &self,
        session_id: &str,
        context: Arc<dyn ContextEngine>,
        cancellation: CancellationToken,
    ) -> Result<Pin<Box<dyn Stream<Item = AgentResponseEvent> + Send>>>;
}
