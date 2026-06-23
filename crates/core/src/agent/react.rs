use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agent::requests::single::AgentEventSink;
use crate::agent::types::{AgentFinishKind, AgentResponseEvent};
use crate::completion::{
    ChatCompletion, ChatCompletionFinishReason, ChatCompletionRequestMessage,
    ChatCompletionResponseChunk,
};
use crate::context::ContextEngine;
use crate::toolbox::{ToolCallEvent, ToolCallEventSink, ToolCallGroupId, Toolbox};
use crate::types::ToolManifest;

pub(crate) async fn run(
    context: Arc<dyn ContextEngine>,
    completion: Arc<dyn ChatCompletion>,
    toolbox: Arc<Toolbox>,
    stream: bool,
    cancellation: CancellationToken,
    sink: Arc<dyn AgentEventSink>,
) -> std::result::Result<(), crate::types::MorayError> {
    let tools = toolbox.list_tools().await;
    if let Err(e) = context.setup(&tools).await {
        warn!(error = %e, "context setup failed");
        sink.emit(AgentResponseEvent::Finished {
            kind: AgentFinishKind::Failed {
                reason: e.to_string(),
            },
        })
        .await;
        return Ok(());
    }
    debug!("setup completed");

    let exit_kind = loop {
        match react_once(
            &context,
            &tools,
            &completion,
            stream,
            &toolbox,
            &cancellation,
            &sink,
        )
        .await
        {
            Ok(nb_tool_calls) => {
                debug!("react loop step completed {{nb_tool_calls={nb_tool_calls}}}");
                if nb_tool_calls == 0 {
                    break AgentFinishKind::Succeeded;
                }
            }
            Err(kind) => break kind,
        }
    };

    if let Err(e) = context.teardown().await {
        warn!(error = %e, "context teardown failed");
    } else {
        debug!("teardown completed");
    }

    sink.emit(AgentResponseEvent::Finished { kind: exit_kind }).await;
    info!("finished");
    Ok(())
}

struct AgentToolCallEventSink {
    sink: Arc<dyn AgentEventSink>,
}

#[async_trait]
impl ToolCallEventSink for AgentToolCallEventSink {
    async fn emit(&self, ev: ToolCallEvent) -> bool {
        self.sink
            .emit(AgentResponseEvent::ToolCall { event: ev })
            .await;
        true
    }
}

fn toolbox_err(kind: impl std::fmt::Display) -> AgentFinishKind {
    AgentFinishKind::Failed {
        reason: kind.to_string(),
    }
}

async fn react_once(
    context: &Arc<dyn ContextEngine>,
    tools: &[ToolManifest],
    completion: &Arc<dyn ChatCompletion>,
    stream: bool,
    toolbox: &Arc<Toolbox>,
    cancellation: &CancellationToken,
    sink: &Arc<dyn AgentEventSink>,
) -> Result<usize, AgentFinishKind> {
    debug!(stream, tool_count = tools.len(), "started");
    let messages = match context.assemble(tools).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "assemble context failed");
            return Err(AgentFinishKind::Failed {
                reason: e.to_string(),
            });
        }
    };
    debug!(message_count = messages.len(), "context assembled");
    let mut chat_stream = match completion.completion(&messages, tools, stream).await {
        Ok(v) => Box::pin(v),
        Err(e) => {
            warn!(error = %e, "completion request failed");
            return Err(AgentFinishKind::Failed {
                reason: e.to_string(),
            });
        }
    };

    sink.as_ref().emit(AgentResponseEvent::Started).await;

    let mut acc_text = String::new();
    let mut tool_call_group: Option<ToolCallGroupId> = None;
    let mut loop_exit: Option<AgentFinishKind> = None;

    'completion: loop {
        let next = tokio::select! {
            _ = cancellation.cancelled() => {
                loop_exit = Some(AgentFinishKind::Canceled);
                break 'completion;
            }
            next = chat_stream.next() => next,
        };

        let chunk = match next {
            None => {
                debug!("chat stream ended without Done chunk");
                break 'completion;
            }
            Some(Ok(chunk)) => chunk,
            Some(Err(e)) => {
                warn!(error = %e, "chat stream returned error chunk");
                loop_exit = Some(AgentFinishKind::Failed {
                    reason: e.to_string(),
                });
                break 'completion;
            }
        };

        sink.as_ref()
            .emit(AgentResponseEvent::CompletionResponse {
                chunk: chunk.clone(),
            })
            .await;

        match chunk {
            ChatCompletionResponseChunk::TextBlock(t) => {
                acc_text.push_str(&t);
            }
            ChatCompletionResponseChunk::Think(_) => {}
            ChatCompletionResponseChunk::ThinkDone => {}
            ChatCompletionResponseChunk::TextDone => {}
            ChatCompletionResponseChunk::ToolCall(tool_call) => {
                debug!(call_id = %tool_call.call_id, tool = %tool_call.name, "received tool call chunk");

                let group = if let Some(id) = tool_call_group {
                    id
                } else {
                    let sink = Arc::new(AgentToolCallEventSink {
                        sink: sink.clone(),
                    });
                    let id = toolbox.begin_group(sink, cancellation.clone()).await;
                    tool_call_group = Some(id);
                    id
                };

                if let Err(e) = toolbox.call_tool(group, tool_call).await {
                    loop_exit = Some(toolbox_err(e));
                    break 'completion;
                }
            }
            ChatCompletionResponseChunk::Done { reason } => match reason {
                ChatCompletionFinishReason::Refusal { reason } => {
                    info!(?reason, "completion refused");
                    loop_exit = Some(AgentFinishKind::Refused { reason });
                    break 'completion;
                }
                ChatCompletionFinishReason::Length => {
                    warn!("completion stopped due to token limit");
                    loop_exit = Some(AgentFinishKind::Failed {
                        reason: "completion stopped due to token limit".to_string(),
                    });
                    break 'completion;
                }
                ChatCompletionFinishReason::Stop => {
                    debug!("completion finished with stop reason");
                    break 'completion;
                }
            },
        }
    }

    if let Some(group) = tool_call_group.take() {
        let end_result = toolbox.end_group(group).await;
        if let Some(kind) = loop_exit {
            let _ = end_result;
            return Err(kind);
        }
        let (tool_call_requests, tool_call_results) = end_result.map_err(toolbox_err)?;

        if cancellation.is_cancelled() {
            return Err(AgentFinishKind::Canceled);
        }

        let nb_tool_calls = tool_call_requests.len();
        info!(nb_tool_calls, "collected tool calls");

        let mut ingest_messages = vec![ChatCompletionRequestMessage::Assistant {
            content: acc_text,
            tool_calls: Some(tool_call_requests),
        }];
        for result in tool_call_results {
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

        info!(nb_tool_calls, "react_once completed");
        return Ok(nb_tool_calls);
    }

    if let Some(kind) = loop_exit {
        return Err(kind);
    }

    debug!("finished without tool calls");
    Ok(0)
}
