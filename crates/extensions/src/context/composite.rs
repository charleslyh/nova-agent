//! In-memory [`ContextEngine`] with an assemble pipeline.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use moray_core::{
    ChatCompletionRequestMessage, ContextEngine, MorayError, ToolManifest,
};

/// One stage in [`CompositeContextEngine::assemble`] (not part of Agent).
///
/// [`Self::bootstrap`] / [`Self::teardown`] bracket one agent run (one user turn): snapshot
/// turn-local state at bootstrap, reuse it across repeated [`Self::process`] calls, then clear
/// on teardown.
pub trait ContextPipelineNode: Send + Sync {
    fn bootstrap(&self) -> Result<(), MorayError> {
        Ok(())
    }

    fn process(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
        tools: &[ToolManifest],
    ) -> Result<Vec<ChatCompletionRequestMessage>, MorayError>;

    fn teardown(&self) -> Result<(), MorayError> {
        Ok(())
    }
}

pub struct CompositeContextEngine {
    messages: RwLock<Vec<ChatCompletionRequestMessage>>,
    pipeline: Vec<Arc<dyn ContextPipelineNode>>,
}

/// Fluent builder for [`CompositeContextEngine`].
#[derive(Default)]
pub struct CompositeContextEngineBuilder {
    messages: Vec<ChatCompletionRequestMessage>,
    pipeline: Vec<Arc<dyn ContextPipelineNode>>,
}

impl CompositeContextEngineBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(mut self, messages: Vec<ChatCompletionRequestMessage>) -> Self {
        self.messages = messages;
        self
    }

    pub fn node(mut self, node: Arc<dyn ContextPipelineNode>) -> Self {
        self.pipeline.push(node);
        self
    }

    pub fn pipeline(mut self, pipeline: Vec<Arc<dyn ContextPipelineNode>>) -> Self {
        self.pipeline = pipeline;
        self
    }

    pub fn build(self) -> CompositeContextEngine {
        CompositeContextEngine {
            messages: RwLock::new(self.messages),
            pipeline: self.pipeline,
        }
    }
}

fn lock_err() -> MorayError {
    MorayError::Message("CompositeContextEngine messages lock poisoned".into())
}

#[async_trait]
impl ContextEngine for CompositeContextEngine {
    async fn bootstrap(&self) -> Result<(), MorayError> {
        for node in &self.pipeline {
            node.bootstrap()?;
        }
        Ok(())
    }

    async fn assemble(
        &self,
        tools: &[ToolManifest],
    ) -> Result<Vec<ChatCompletionRequestMessage>, MorayError> {
        let mut messages = self.messages.read().map_err(|_| lock_err())?.clone();
        for node in &self.pipeline {
            messages = node.process(messages, tools)?;
        }
        Ok(messages)
    }

    async fn ingest(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
    ) -> Result<(), MorayError> {
        self.messages
            .write()
            .map_err(|_| lock_err())?
            .extend(messages);
        Ok(())
    }

    async fn teardown(&self) -> Result<(), MorayError> {
        for node in self.pipeline.iter().rev() {
            node.teardown()?;
        }
        Ok(())
    }

    async fn clear(&self) -> Result<(), MorayError> {
        self.messages.write().map_err(|_| lock_err())?.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use moray_core::ChatCompletionRequestMessage;

    use crate::preambles::TemplatedPreamblerBuilder;

    use super::*;

    #[tokio::test]
    async fn assemble_runs_pipeline() {
        let engine = CompositeContextEngineBuilder::new()
            .messages(vec![ChatCompletionRequestMessage::User {
                content: "hi".into(),
            }])
            .node(Arc::new(
                TemplatedPreamblerBuilder::new()
                    .template("## Character\n\n{{character}}\n")
                    .with_string("character", "test")
                    .build(),
            ))
            .build();
        engine.bootstrap().await.expect("bootstrap");
        let out = engine.assemble(&[]).await.expect("assemble");
        assert!(matches!(
            out.first(),
            Some(ChatCompletionRequestMessage::System { .. })
        ));
        assert_eq!(out.len(), 2);
    }

    struct LifecycleNode {
        bootstrap_calls: AtomicUsize,
        teardown_calls: AtomicUsize,
    }

    impl ContextPipelineNode for LifecycleNode {
        fn bootstrap(&self) -> Result<(), MorayError> {
            self.bootstrap_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn process(
            &self,
            messages: Vec<ChatCompletionRequestMessage>,
            _tools: &[ToolManifest],
        ) -> Result<Vec<ChatCompletionRequestMessage>, MorayError> {
            Ok(messages)
        }

        fn teardown(&self) -> Result<(), MorayError> {
            self.teardown_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn bootstrap_and_teardown_forward_to_pipeline_nodes() {
        let first = Arc::new(LifecycleNode {
            bootstrap_calls: AtomicUsize::new(0),
            teardown_calls: AtomicUsize::new(0),
        });
        let second = Arc::new(LifecycleNode {
            bootstrap_calls: AtomicUsize::new(0),
            teardown_calls: AtomicUsize::new(0),
        });
        let engine = CompositeContextEngineBuilder::new()
            .node(first.clone())
            .node(second.clone())
            .build();
        engine.bootstrap().await.expect("bootstrap");
        assert_eq!(first.bootstrap_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second.bootstrap_calls.load(Ordering::SeqCst), 1);
        engine.teardown().await.expect("teardown");
        assert_eq!(first.teardown_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second.teardown_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn engine_bootstrap_freezes_preamble_across_assembles() {
        use std::sync::RwLock;

        let current = Arc::new(RwLock::new("first".to_string()));
        let current_in_fn = current.clone();
        let preambler = Arc::new(
            TemplatedPreamblerBuilder::new()
                .template("## Character\n\n{{character}}\n")
                .with_fn("character", move || current_in_fn.read().expect("lock").clone())
                .build(),
        );
        let engine = CompositeContextEngineBuilder::new()
            .messages(vec![ChatCompletionRequestMessage::User {
                content: "hi".into(),
            }])
            .node(preambler)
            .build();
        engine.bootstrap().await.expect("bootstrap");
        *current.write().expect("lock") = "second".into();
        let first = engine.assemble(&[]).await.expect("assemble 1");
        let second = engine.assemble(&[]).await.expect("assemble 2");
        let system_content = |msgs: &[ChatCompletionRequestMessage]| {
            let ChatCompletionRequestMessage::System { content } = msgs.first().expect("system") else {
                panic!("expected system");
            };
            content.clone()
        };
        assert!(system_content(&first).contains("first"));
        assert!(system_content(&second).contains("first"));
        assert!(!system_content(&second).contains("second"));
    }
}
