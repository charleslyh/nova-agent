//! In-memory [`ContextEngine`] with an assemble pipeline.

use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use moray_core::{
    ChatCompletionRequestMessage, ContextEngine, MorayError, ToolManifest,
};

use super::preamble::PreambleProvider;

/// One stage in [`CompositeContextEngine::assemble`] (not part of Agent).
///
/// [`Self::setup`] / [`Self::teardown`] bracket one agent run (one user turn): snapshot turn-local
/// state at run start, reuse it across repeated [`Self::process`] calls, then clear on run end.
/// Names match [`ContextEngine::setup`] / [`ContextEngine::teardown`] on the engine.
///
/// Pipeline nodes MUST NOT inject the turn system preamble; the engine freezes it at
/// [`ContextEngine::setup`] via [`PreambleProvider`] and prepends it on [`ContextEngine::assemble`].
pub trait ContextPipelineNode: Send + Sync {
    fn setup(
        &self,
        _tools: &[ToolManifest],
        _preamble: Option<&str>,
        _transcript: &[ChatCompletionRequestMessage],
    ) -> Result<(), MorayError> {
        Ok(())
    }

    /// Mutates the transcript in place during [`ContextEngine::assemble`].
    fn process(
        &self,
        // `&mut`: `CompositeContextEngine::assemble` already owns the buffer via `mem::take`.
        // Not `Vec` in/out — would move the whole `Vec` per node with no gain for in-place edits.
        // Not consume-only `mut Vec` — ownership cannot return to `assemble` on `?`, breaking
        // error recovery and write-back to the engine store.
        transcript: &mut Vec<ChatCompletionRequestMessage>,
        tools: &[ToolManifest],
    ) -> Result<(), MorayError>;

    /// Notified after new messages are appended to the transcript. Read-only: nodes MUST NOT
    /// mutate the store here; compaction and other transforms belong in [`Self::process`].
    fn on_ingest(
        &self,
        _ingested: &[ChatCompletionRequestMessage],
    ) -> Result<(), MorayError> {
        Ok(())
    }

    fn teardown(&self) -> Result<(), MorayError> {
        Ok(())
    }
}

pub struct CompositeContextEngine {
    /// User / assistant / tool messages only (no turn system preamble).
    transcript: RwLock<Vec<ChatCompletionRequestMessage>>,
    /// System prompt frozen for the current agent run ([`ContextEngine::setup`]).
    turn_preamble: RwLock<Option<String>>,
    preamble_provider: Option<Arc<dyn PreambleProvider>>,
    pipeline: Vec<Arc<dyn ContextPipelineNode>>,
}

/// Fluent builder for [`CompositeContextEngine`].
#[derive(Default)]
pub struct CompositeContextEngineBuilder {
    messages: Vec<ChatCompletionRequestMessage>,
    preamble_provider: Option<Arc<dyn PreambleProvider>>,
    pipeline: Vec<Arc<dyn ContextPipelineNode>>,
}

impl CompositeContextEngineBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(mut self, messages: Vec<ChatCompletionRequestMessage>) -> Self {
        #[cfg(debug_assertions)]
        ensure_transcript_messages(&messages)
            .expect("CompositeContextEngineBuilder::messages");
        self.messages = messages;
        self
    }

    pub fn preamble(mut self, provider: Arc<dyn PreambleProvider>) -> Self {
        self.preamble_provider = Some(provider);
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
            transcript: RwLock::new(self.messages),
            turn_preamble: RwLock::new(None),
            preamble_provider: self.preamble_provider,
            pipeline: self.pipeline,
        }
    }
}

#[async_trait]
impl ContextEngine for CompositeContextEngine {
    async fn setup(&self, tools: &[ToolManifest]) -> Result<(), MorayError> {
        let transcript = self.transcript.read().map_err(|_| lock_err())?;

        // Generate and freeze the system prompt for the current agent run.
        if let Some(provider) = &self.preamble_provider {
            let content = provider.generate(&transcript, tools)?;
            *self
                .turn_preamble
                .write()
                .map_err(|_| preamble_lock_err())? = Some(content);
        }

        let preamble_guard = self
            .turn_preamble
            .read()
            .map_err(|_| preamble_lock_err())?;
        let preamble = preamble_guard.as_deref();

        // Give all pipeline nodes a chance to setup. Such as calculating tokens, etc.
        for node in &self.pipeline {
            node.setup(tools, preamble, &transcript)?;
        }

        Ok(())
    }

    async fn assemble(
        &self,
        tools: &[ToolManifest],
    ) -> Result<Vec<ChatCompletionRequestMessage>, MorayError> {
        let mut guard = self.transcript.write().map_err(|_| lock_err())?;
        let mut transcript = std::mem::take(&mut *guard);
        #[cfg(debug_assertions)]
        debug_assert!(ensure_transcript_messages(&transcript).is_ok());

        for node in &self.pipeline {
            if let Err(e) = node.process(&mut transcript, tools) {
                *guard = transcript;
                return Err(e);
            }
        }

        *guard = transcript.clone();

        if let Some(preamble) = self
            .turn_preamble
            .read()
            .ok()
            .and_then(|p| p.clone())
        {
            transcript.insert(
                0,
                ChatCompletionRequestMessage::System {
                    content: preamble,
                },
            );
        }

        Ok(transcript)
    }

    async fn ingest(
        &self,
        messages: Vec<ChatCompletionRequestMessage>,
    ) -> Result<(), MorayError> {
        #[cfg(debug_assertions)]
        ensure_transcript_messages(&messages)?;

        self.transcript
            .write()
            .map_err(|_| lock_err())?
            .extend_from_slice(&messages);

        // 将新消息通知给所有 pipeline nodes，使它们可以针对 delta 进行处理，如果 node 实现为状态机，则可以避免每次都重新计算一次。
        for node in &self.pipeline {
            node.on_ingest(&messages)?;
        }

        Ok(())
    }

    async fn teardown(&self) -> Result<(), MorayError> {
        for node in self.pipeline.iter().rev() {
            node.teardown()?;
        }

        *self
            .turn_preamble
            .write()
            .map_err(|_| preamble_lock_err())? = None;

        Ok(())
    }

    async fn clear(&self) -> Result<(), MorayError> {
        self.transcript.write().map_err(|_| lock_err())?.clear();
        Ok(())
    }

    fn snapshot(&self) -> Option<Vec<ChatCompletionRequestMessage>> {
        self.transcript.read().ok().map(|g| g.clone())
    }
}

#[cfg(debug_assertions)]
const TRANSCRIPT_NO_SYSTEM_MSG: &str =
    "transcript must not contain System messages; turn preamble is prepended at assemble";

/// Transcript must be user / assistant / tool only; turn preamble is prepended at assemble.
#[cfg(debug_assertions)]
fn ensure_transcript_messages(messages: &[ChatCompletionRequestMessage]) -> Result<(), MorayError> {
    if messages
        .iter()
        .any(|m| matches!(m, ChatCompletionRequestMessage::System { .. }))
    {
        return Err(MorayError::Message(TRANSCRIPT_NO_SYSTEM_MSG.into()));
    }
    Ok(())
}

fn lock_err() -> MorayError {
    MorayError::Message("CompositeContextEngine transcript lock poisoned".into())
}

fn preamble_lock_err() -> MorayError {
    MorayError::Message("CompositeContextEngine turn_preamble lock poisoned".into())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use moray_core::ChatCompletionRequestMessage;

    use crate::preambles::TemplatedPreamblerBuilder;

    use super::*;

    #[tokio::test]
    async fn assemble_prepends_preamble_without_storing_system_in_transcript() {
        let engine = CompositeContextEngineBuilder::new()
            .messages(vec![ChatCompletionRequestMessage::User {
                content: "hi".into(),
            }])
            .preamble(Arc::new(
                TemplatedPreamblerBuilder::new()
                    .template("## Character\n\n{{character}}\n")
                    .with_string("character", "test")
                    .build(),
            ))
            .build();
        engine.setup(&[]).await.expect("setup");
        let out = engine.assemble(&[]).await.expect("assemble");
        assert!(matches!(
            out.first(),
            Some(ChatCompletionRequestMessage::System { .. })
        ));
        assert_eq!(out.len(), 2);

        let store = engine.transcript.read().expect("read");
        assert_eq!(store.len(), 1);
        assert!(matches!(store[0], ChatCompletionRequestMessage::User { .. }));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(
        expected = "transcript must not contain System messages; turn preamble is prepended at assemble"
    )]
    fn messages_builder_rejects_system_in_transcript() {
        CompositeContextEngineBuilder::new()
            .messages(vec![
                ChatCompletionRequestMessage::System {
                    content: "legacy".into(),
                },
                ChatCompletionRequestMessage::User {
                    content: "hi".into(),
                },
            ])
            .build();
    }

    #[tokio::test]
    #[cfg(debug_assertions)]
    async fn ingest_rejects_system_messages() {
        let engine = CompositeContextEngineBuilder::new().build();
        let err = engine
            .ingest(vec![ChatCompletionRequestMessage::System {
                content: "oops".into(),
            }])
            .await
            .expect_err("ingest");
        assert!(matches!(
            err,
            MorayError::Message(ref m) if m == TRANSCRIPT_NO_SYSTEM_MSG
        ));
    }

    struct LifecycleNode {
        setup_calls: AtomicUsize,
        teardown_calls: AtomicUsize,
        ingest_calls: AtomicUsize,
    }

    impl ContextPipelineNode for LifecycleNode {
        fn setup(
            &self,
            _tools: &[ToolManifest],
            preamble: Option<&str>,
            _transcript: &[ChatCompletionRequestMessage],
        ) -> Result<(), MorayError> {
            self.setup_calls.fetch_add(1, Ordering::SeqCst);
            assert!(preamble.is_none());
            Ok(())
        }

        fn process(
            &self,
            _transcript: &mut Vec<ChatCompletionRequestMessage>,
            _tools: &[ToolManifest],
        ) -> Result<(), MorayError> {
            Ok(())
        }

        fn on_ingest(
            &self,
            _ingested: &[ChatCompletionRequestMessage],
        ) -> Result<(), MorayError> {
            self.ingest_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn teardown(&self) -> Result<(), MorayError> {
            self.teardown_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn setup_and_teardown_forward_to_pipeline_nodes() {
        let first = Arc::new(LifecycleNode {
            setup_calls: AtomicUsize::new(0),
            teardown_calls: AtomicUsize::new(0),
            ingest_calls: AtomicUsize::new(0),
        });
        let second = Arc::new(LifecycleNode {
            setup_calls: AtomicUsize::new(0),
            teardown_calls: AtomicUsize::new(0),
            ingest_calls: AtomicUsize::new(0),
        });
        let engine = CompositeContextEngineBuilder::new()
            .node(first.clone())
            .node(second.clone())
            .build();
        engine.setup(&[]).await.expect("setup");
        assert_eq!(first.setup_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second.setup_calls.load(Ordering::SeqCst), 1);
        engine.teardown().await.expect("teardown");
        assert_eq!(first.teardown_calls.load(Ordering::SeqCst), 1);
        assert_eq!(second.teardown_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ingest_notifies_pipeline_nodes() {
        let node = Arc::new(LifecycleNode {
            setup_calls: AtomicUsize::new(0),
            teardown_calls: AtomicUsize::new(0),
            ingest_calls: AtomicUsize::new(0),
        });
        let engine = CompositeContextEngineBuilder::new().node(node.clone()).build();
        engine
            .ingest(vec![ChatCompletionRequestMessage::User {
                content: "hi".into(),
            }])
            .await
            .expect("ingest");
        assert_eq!(node.ingest_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn engine_setup_freezes_preamble_across_assembles() {
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
            .preamble(preambler)
            .build();
        engine.setup(&[]).await.expect("setup");
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
