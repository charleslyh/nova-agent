use async_trait::async_trait;

use crate::completion::ChatCompletionRequestMessage;
use crate::types::{MorayError, ToolManifest};

#[async_trait]
pub trait ContextEngine: Send + Sync {
    async fn setup(&self, tools: &[ToolManifest]) -> Result<(), MorayError>;

    async fn assemble(
        &self,
        tools: &[ToolManifest],
    ) -> Result<Vec<ChatCompletionRequestMessage>, MorayError>;

    async fn ingest(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
    ) -> Result<(), MorayError>;

    async fn teardown(&self) -> Result<(), MorayError>;

    async fn clear(&self) -> Result<(), MorayError>;

    /// Returns a copy of ingested transcript messages when supported.
    fn snapshot(&self) -> Option<Vec<ChatCompletionRequestMessage>> {
        None
    }
}
