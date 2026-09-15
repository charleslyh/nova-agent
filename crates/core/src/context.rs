use async_trait::async_trait;

use crate::completion::ChatCompletionRequestMessage;
use crate::types::{NovaError, ToolManifest};

#[async_trait]
pub trait ContextEngine: Send + Sync {
    async fn setup(&self, tools: &[ToolManifest]) -> Result<(), NovaError>;

    async fn assemble(
        &self,
        tools: &[ToolManifest],
    ) -> Result<Vec<ChatCompletionRequestMessage>, NovaError>;

    async fn ingest(&self, messages: Vec<ChatCompletionRequestMessage>) -> Result<(), NovaError>;

    async fn teardown(&self) -> Result<(), NovaError>;

    async fn clear(&self) -> Result<(), NovaError>;

    /// Returns a clone of ingested transcript messages when supported.
    ///
    /// [`ChatCompletionRequestMessage`] is `Clone`; implementations should return
    /// owned copies (for example `Some(inner.messages.clone())`).
    ///
    /// Intentionally synchronous: implementations are expected to use in-process
    /// locks (e.g. `std::sync::RwLock`) rather than async runtime primitives.
    fn snapshot(&self) -> Option<Vec<ChatCompletionRequestMessage>> {
        None
    }
}
