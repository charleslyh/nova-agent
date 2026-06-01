use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use futures::stream::select_all;
use futures::Stream;
use futures::StreamExt;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn, Instrument};

use crate::completion::{
    ChatCompletion, ChatCompletionFinishReason, ChatCompletionRequestMessage,
    ChatCompletionResponseChunk,
};
use crate::context::ContextEngine;
use crate::toolbox::{ToolCallEvent, Toolbox};
use crate::types::MorayError;
use crate::types::{ToolCallRequest, ToolCallResult};

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AgentFinishKind {
    Succeeded,
    Canceled,
    Refused {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        reason: Option<String>,
    },
    Failed {
        reason: String,
    },
}

/// Keeps the run stream narrowly scoped so transport and UI layers can
/// remain stable even when internals evolve.
///
/// `Eq` is intentionally not derived because `ToolCallEvent::Custom`
/// can contain `serde_json::Value`, which is not `Eq`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "snake_case"))]
pub enum AgentResponseEvent {
    Started,
    CompletionResponse { chunk: ChatCompletionResponseChunk },
    ToolCall { event: ToolCallEvent },
    Finished { kind: AgentFinishKind },
}

pub fn agent_run(
    context: Arc<dyn ContextEngine>,
    toolbox: Arc<Toolbox>,
    completion: Arc<dyn ChatCompletion>,
    stream: bool,
    cancellation: CancellationToken,
) -> Result<Pin<Box<dyn Stream<Item = AgentResponseEvent> + Send>>, MorayError> {
    let (tx, rx) = unbounded_channel::<AgentResponseEvent>();
    let run_span = tracing::info_span!("agent.run");
    info!(parent: &run_span, "started");

    tokio::spawn(
        agent_run_impl(
            context,
            toolbox,
            completion,
            stream,
            cancellation,
            tx,
        )
        .instrument(run_span),
    );

    Ok(Box::pin(UnboundedReceiverStream::new(rx)))
}

async fn agent_run_impl(
    context: Arc<dyn ContextEngine>,
    toolbox: Arc<Toolbox>,
    completion: Arc<dyn ChatCompletion>,
    stream: bool,
    cancellation: CancellationToken,
    tx: UnboundedSender<AgentResponseEvent>,
) {
    if let Err(e) = context.bootstrap().await {
        warn!(error = %e, "context bootstrap failed");
        resposne(
            &tx,
            AgentResponseEvent::Finished {
                kind: AgentFinishKind::Failed {
                    reason: e.to_string(),
                },
            },
        );
        return;
    }
    debug!("bootstrap completed");

    let exit_kind = loop {
        match react_once(
            &completion,
            &toolbox,
            &context,
            stream,
            &tx,
            &cancellation,
        )
        .await
        {
            Ok(nb_tool_calls) => {
                debug!("react loop step completed {{nb_tool_calls={nb_tool_calls}}}");
                if nb_tool_calls == 0 {
                    // No pending tool work is treated as a fixed point; continuing
                    // the loop would only re-ask the model without new evidence.
                    break AgentFinishKind::Succeeded;
                }
            }

            // Surface failures through the same stream contract so callers do not
            // need out-of-band error channels.
            Err(kind) => break kind,
        }
    };

    if let Err(e) = context.teardown().await {
        warn!(error = %e, "context teardown failed");
    } else {
        debug!("teardown completed");
    }

    resposne(&tx, AgentResponseEvent::Finished { kind: exit_kind });
    info!("finished");
}

/// Builds arguments for [`Agent::run`] and forwards to it. [`Self::completion`] and
/// [`Self::context`] are required; other fields use in-builder defaults (empty toolbox with no
/// authorizer, streaming on, fresh cancellation token).
pub struct AgentRequestBuilder {
    completion: Option<Arc<dyn ChatCompletion>>,
    context: Option<Arc<dyn ContextEngine>>,
    toolbox: Option<Arc<Toolbox>>,
    stream: bool,
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

    pub fn cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    pub fn run(self) -> Result<Pin<Box<dyn Stream<Item = AgentResponseEvent> + Send>>, MorayError> {
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

        agent_run(
            context,
            toolbox,
            completion,
            self.stream,
            cancellation,
        )
    }
}

fn empty_toolbox() -> Arc<Toolbox> {
    Arc::new(Toolbox::new(HashMap::new(), Vec::new(), None))
}

fn resposne(tx: &UnboundedSender<AgentResponseEvent>, ev: AgentResponseEvent) {
    let _ = tx.send(ev);
}

/// Groups tool streams so execution stays concurrent while emissions remain
/// ordered through one forwarding path.
struct ToolCallGroup {
    toolbox: Arc<Toolbox>,
    tx: UnboundedSender<AgentResponseEvent>,
    merged: futures::stream::SelectAll<Pin<Box<dyn Stream<Item = ToolCallEvent> + Send>>>,
}

impl ToolCallGroup {
    fn new(toolbox: Arc<Toolbox>, tx: UnboundedSender<AgentResponseEvent>) -> Self {
        Self {
            toolbox,
            tx,
            merged: select_all(Vec::new()),
        }
    }

    #[instrument(
        name = "agent.spawn_tool_call",
        level = "debug",
        skip(self, tc),
        fields(call_id = %tc.call_id, tool = %tc.name)
    )]
    fn spawn_tool_call(&mut self, tc: ToolCallRequest) {
        debug!("spawned");
        let stream = Arc::clone(&self.toolbox).call_tool(&tc.call_id, &tc.name, &tc.arguments);
        self.merged.push(stream);
    }

    #[instrument(name = "agent.join_tool_calls", level = "debug", skip(self))]
    async fn join(mut self) -> Vec<ToolCallResult> {
        let mut results = Vec::new();

        while let Some(tool_ev) = self.merged.next().await {
            if let ToolCallEvent::Completed { content } = &tool_ev {
                debug!(call_id = %content.call_id, "finished");
                results.push(content.clone());
            }

            resposne(&self.tx, AgentResponseEvent::ToolCall { event: tool_ev });
        }

        results
    }
}

async fn react_once(
    completion: &Arc<dyn ChatCompletion>,
    toolbox: &Arc<Toolbox>,
    context: &Arc<dyn ContextEngine>,
    stream: bool,
    tx: &UnboundedSender<AgentResponseEvent>,
    cancellation: &CancellationToken,
) -> Result<usize, AgentFinishKind> {
    let tools = toolbox.list_tools().await;
    debug!(stream, tool_count = tools.len(), "started");
    let messages = match context.assemble(&tools).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "assemble context failed");
            return Err(AgentFinishKind::Failed {
                reason: e.to_string(),
            });
        }
    };
    debug!(message_count = messages.len(), "context assembled");
    let mut chat_stream = match completion.completion(&messages, &tools, stream).await {
        Ok(v) => Box::pin(v),
        Err(e) => {
            // Fail fast to avoid a loop that appears alive but can no longer produce model output.
            warn!(error = %e, "completion request failed");
            return Err(AgentFinishKind::Failed {
                reason: e.to_string(),
            });
        }
    };

    resposne(tx, AgentResponseEvent::Started);

    let mut acc_text = String::new();
    let mut acc_tool_calls: Vec<ToolCallRequest> = Vec::new();
    let mut tool_call_group = ToolCallGroup::new(Arc::clone(toolbox), tx.clone());

    loop {
        // Race cancellation with model chunks so shutdown latency is not coupled to provider chunk cadence or backpressure.
        let next = tokio::select! {
            _ = cancellation.cancelled() => return Err(AgentFinishKind::Canceled),
            next = chat_stream.next() => next,
        };

        let chunk = match next {
            // Early EOF is treated as a safe boundary to avoid replaying partial intent as if it were complete.
            None => {
                debug!("chat stream ended without Done chunk");
                break;
            }
            Some(Ok(chunk)) => chunk,
            Some(Err(e)) => {
                warn!(error = %e, "chat stream returned error chunk");
                return Err(AgentFinishKind::Failed {
                    reason: e.to_string(),
                });
            }
        };

        resposne(
            tx,
            AgentResponseEvent::CompletionResponse {
                chunk: chunk.clone(),
            },
        );

        match chunk {
            ChatCompletionResponseChunk::TextBlock(t) => {
                acc_text.push_str(&t);
            }
            ChatCompletionResponseChunk::Think(_) => {}
            ChatCompletionResponseChunk::ThinkDone => {}
            ChatCompletionResponseChunk::TextDone => {}
            ChatCompletionResponseChunk::ToolCall(tc) => {
                debug!(call_id = %tc.call_id, tool = %tc.name, "received tool call chunk");
                acc_tool_calls.push(tc.clone());
                tool_call_group.spawn_tool_call(tc);
            }
            ChatCompletionResponseChunk::Done { reason } => match reason {
                ChatCompletionFinishReason::Refusal { reason } => {
                    info!(?reason, "completion refused");
                    return Err(AgentFinishKind::Refused { reason });
                }
                ChatCompletionFinishReason::Length => {
                    warn!("completion stopped due to token limit");
                    return Err(AgentFinishKind::Failed {
                        reason: "completion stopped due to token limit".to_string(),
                    });
                }
                ChatCompletionFinishReason::Stop => {
                    debug!("completion finished with stop reason");
                    break;
                }
            },
        }
    }

    if acc_tool_calls.is_empty() {
        debug!("finished without tool calls");
        return Ok(0);
    }

    let tool_results = tokio::select! {
        _ = cancellation.cancelled() => return Err(AgentFinishKind::Canceled),
        result = tool_call_group.join() => result,
    };

    let nb_tool_calls = acc_tool_calls.len();
    info!(nb_tool_calls, "collected tool calls");

    let mut ingest_messages = vec![ChatCompletionRequestMessage::Assistant {
        content: acc_text,
        tool_calls: (!acc_tool_calls.is_empty()).then_some(acc_tool_calls),
    }];
    for result in tool_results {
        ingest_messages.push(ChatCompletionRequestMessage::Tool {
            content: result.content,
            call_id: result.call_id,
        });
    }

    if let Err(e) = context.ingest(ingest_messages).await {
        warn!(error = %e, "failed to ingest react_once output");
        return Err(AgentFinishKind::Failed {
            reason: e.to_string(),
        });
    }

    debug!("ingest succeeded");
    Ok(nb_tool_calls)
}
