use std::sync::Arc;

use async_trait::async_trait;
use moray_core::ContextEngine;
use tokio_util::sync::CancellationToken;

use crate::{Result, SessionEventSink};

#[async_trait]
pub trait AgentRunner: Send + Sync {
    async fn run_turn(
        &self,
        session_id: &str,
        context: Arc<dyn ContextEngine>,
        cancellation: CancellationToken,
        sink: Arc<dyn SessionEventSink>,
    ) -> Result<()>;
}
