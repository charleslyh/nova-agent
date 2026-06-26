use std::sync::Arc;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, Instrument};

use crate::agent::react;
use crate::agent::requests::single::AgentEventSink;
use crate::completion::ChatCompletion;
use crate::context::ContextEngine;
use crate::toolbox::Toolbox;
use crate::types::MorayError;

const DEFAULT_MAX_ROUNDS: usize = 10;

/// Builds and spawns a single-agent ReAct run. [`Self::completion`] and [`Self::context`] are required.
pub struct AgentRequestBuilder {
    completion: Option<Arc<dyn ChatCompletion>>,
    context: Option<Arc<dyn ContextEngine>>,
    toolbox: Option<Arc<Toolbox>>,
    stream: bool,
    max_rounds: usize,
    cancellation: Option<CancellationToken>,
}

impl Default for AgentRequestBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRequestBuilder {
    pub fn new() -> Self {
        Self {
            completion: None,
            context: None,
            toolbox: None,
            cancellation: None,
            stream: true,
            max_rounds: DEFAULT_MAX_ROUNDS,
        }
    }

    pub fn completion(mut self, completion: Arc<dyn ChatCompletion>) -> Self {
        self.completion = Some(completion);
        self
    }

    pub fn context(mut self, context: Arc<dyn ContextEngine>) -> Self {
        self.context = Some(context);
        self
    }

    pub fn toolbox(mut self, toolbox: Arc<Toolbox>) -> Self {
        self.toolbox = Some(toolbox);
        self
    }

    pub fn stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    pub fn max_rounds(mut self, max_rounds: usize) -> Self {
        self.max_rounds = max_rounds;
        self
    }

    pub fn cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub fn run(
        self,
        sink: Arc<dyn AgentEventSink>,
    ) -> std::result::Result<JoinHandle<std::result::Result<(), MorayError>>, MorayError> {
        let Some(completion) = self.completion else {
            return Err(MorayError::Message(
                "AgentRequestBuilder: missing required `completion`".into(),
            ));
        };
        let Some(context) = self.context else {
            return Err(MorayError::Message(
                "AgentRequestBuilder: missing required `context`".into(),
            ));
        };

        let toolbox = self.toolbox.unwrap_or_else(empty_toolbox);
        let cancellation = self.cancellation.unwrap_or_default();

        info!(stream = self.stream, max_rounds = self.max_rounds, "agent request started");
        Ok(tokio::spawn(
            async move {
                react::run(
                    context,
                    completion,
                    toolbox,
                    self.stream,
                    self.max_rounds,
                    cancellation,
                    sink,
                )
                .await
            }
            .in_current_span(),
        ))
    }
}

fn empty_toolbox() -> Arc<Toolbox> {
    Arc::new(Toolbox::new(
        std::collections::HashMap::new(),
        Vec::new(),
        None,
    ))
}
