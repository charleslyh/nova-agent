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
use crate::toolbox::{ToolCallEvent, ToolCallEventSink, ToolCallGroupId, Toolbox, ToolboxError};
use crate::types::{ToolCallRequest, ToolCallStatus, ToolManifest};

/// Prompt injected as a User message when the agent exceeds its configured
/// round limit. Instructs the LLM to produce a text-only summary without
/// attempting any tool calls.
const ROUND_LIMIT_PROMPT: &str = "\
CRITICAL: The maximum number of agent steps has been reached. \
Tools are disabled until the next user input. \
Respond with text only and do not attempt any tool calls.\n\n\
Provide the best possible final answer using only the conversation \
and tool results already available. Briefly state that the agent step \
limit was reached, summarize what was accomplished, disclose any \
unfinished work, and give actionable next steps. \
Do not claim that unverified work was completed.";

pub(crate) async fn run(
    context: Arc<dyn ContextEngine>,
    completion: Arc<dyn ChatCompletion>,
    toolbox: Arc<Toolbox>,
    stream: bool,
    max_rounds: usize,
    cancellation: CancellationToken,
    sink: Arc<dyn AgentEventSink>,
) -> std::result::Result<(), crate::types::NovaError> {
    let tools = toolbox.list_tools().await;
    info!(stream, tool_count = tools.len(), "react run started");
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

    let mut round = 0usize;
    let exit_kind = loop {
        round += 1;

        // --- Round-limit graceful wrap-up ---
        if round > max_rounds {
            warn!(
                max_rounds,
                round, "react run exceeded max rounds, entering wrap-up"
            );

            // Inject the round-limit prompt so the LLM knows tools are disabled.
            if let Err(e) = context
                .ingest(vec![ChatCompletionRequestMessage::User {
                    content: ROUND_LIMIT_PROMPT.to_string(),
                }])
                .await
            {
                warn!(error = %e, "failed to inject round-limit prompt");
                break AgentFinishKind::Failed {
                    reason: e.to_string(),
                };
            }

            // Execute one final completion with NO tools (physically prevents tool calls).
            match react_once(
                &context,
                &[], // empty tools — LLM cannot produce tool calls
                &completion,
                stream,
                &toolbox,
                &cancellation,
                &sink,
            )
            .await
            {
                Ok(_) => {
                    info!("wrap-up round completed");
                }
                Err(kind) => {
                    warn!(?kind, "wrap-up round failed, degrading to error");
                    break kind;
                }
            }

            break AgentFinishKind::RoundLimitReached;
        }

        // --- Normal react step ---
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

    info!(?exit_kind, "react run finished");
    sink.emit(AgentResponseEvent::Finished { kind: exit_kind })
        .await;
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

/// Emit the complete event sequence for a call that failed before the toolbox
/// could start it (e.g. an unknown tool). The toolbox only emits events for
/// calls it accepted, so without this the failure exists only at the turn
/// level: consumers match a call's events by `call_id`, and a pending entry
/// that never sees `Finished` stays pending forever.
async fn emit_unstarted_call_events(
    sink: &Arc<dyn AgentEventSink>,
    tool_call: &ToolCallRequest,
    error: &ToolboxError,
) {
    let sink = AgentToolCallEventSink { sink: sink.clone() };
    let call_id = tool_call.call_id.clone();
    let _ = sink
        .emit(ToolCallEvent::requested(
            call_id.clone(),
            tool_call.name.clone(),
            tool_call.arguments.clone(),
        ))
        .await;
    // The reason rides as payload so it renders on the call's own surface,
    // not only in the turn-level failure.
    let _ = sink
        .emit(ToolCallEvent::payload(call_id.clone(), error.to_string()))
        .await;
    let _ = sink
        .emit(ToolCallEvent::finished(call_id, ToolCallStatus::Error))
        .await;
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
                info!("turn canceled");
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
                    let sink = Arc::new(AgentToolCallEventSink { sink: sink.clone() });
                    let id = toolbox.begin_group(sink, cancellation.clone()).await;
                    tool_call_group = Some(id);
                    id
                };

                if let Err(e) = toolbox.call_tool(group, tool_call.clone()).await {
                    warn!(
                        call_id = %tool_call.call_id,
                        tool = %tool_call.name,
                        error = %e,
                        "tool call failed"
                    );
                    emit_unstarted_call_events(sink, &tool_call, &e).await;
                    loop_exit = Some(toolbox_err(e));
                    break 'completion;
                }
            }
            ChatCompletionResponseChunk::Done { reason, .. } => match reason {
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

    let (nb_tool_calls, ingest_messages) = if let Some(group) = tool_call_group.take() {
        let end_result = toolbox.end_group(group).await;
        let (tool_call_requests, tool_call_results) = match end_result {
            Ok(v) => v,
            Err(e) => {
                if let Some(kind) = loop_exit {
                    debug!(?kind, "react_once exiting early with pending tool group");
                    return Err(kind);
                }
                return Err(toolbox_err(e));
            }
        };

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

        (nb_tool_calls, ingest_messages)
    } else {
        (
            0,
            vec![ChatCompletionRequestMessage::Assistant {
                content: acc_text,
                tool_calls: None,
            }],
        )
    };

    if let Err(e) = context.ingest(ingest_messages).await {
        warn!(error = %e, "failed to ingest react_once output");
        return Err(AgentFinishKind::Failed {
            reason: e.to_string(),
        });
    }

    // After ingesting the assistant message (and tool results, if any) so the
    // model can see the turn's final output — including a canceled tool's
    // final message — in the next turn, honor a pending cancellation / loop
    // exit. This is the single exit point for loop_exit: both the
    // tool-call-group path and the no-tool-call path funnel through here.
    if let Some(kind) = loop_exit {
        return Err(kind);
    }
    if cancellation.is_cancelled() {
        return Err(AgentFinishKind::Canceled);
    }

    if nb_tool_calls > 0 {
        info!(nb_tool_calls, "react_once completed");
    } else {
        debug!("finished without tool calls");
    }
    Ok(nb_tool_calls)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolbox::{ToolCallEventKind, ToolboxBuilder};
    use crate::types::NovaError;
    use futures::{stream, Stream};
    use std::pin::Pin;
    use std::sync::Mutex;

    struct NoopContext;

    #[async_trait]
    impl ContextEngine for NoopContext {
        async fn setup(&self, _tools: &[ToolManifest]) -> Result<(), NovaError> {
            Ok(())
        }
        async fn assemble(
            &self,
            _tools: &[ToolManifest],
        ) -> Result<Vec<ChatCompletionRequestMessage>, NovaError> {
            Ok(Vec::new())
        }
        async fn ingest(
            &self,
            _messages: Vec<ChatCompletionRequestMessage>,
        ) -> Result<(), NovaError> {
            Ok(())
        }
        async fn teardown(&self) -> Result<(), NovaError> {
            Ok(())
        }
        async fn clear(&self) -> Result<(), NovaError> {
            Ok(())
        }
    }

    /// One round: the model calls a tool the toolbox does not have, then stops.
    struct HallucinatedToolCall;

    #[async_trait]
    impl ChatCompletion for HallucinatedToolCall {
        async fn completion(
            &self,
            _messages: &[ChatCompletionRequestMessage],
            _tools: &[ToolManifest],
            _stream: bool,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<ChatCompletionResponseChunk, NovaError>> + Send>>,
            NovaError,
        > {
            let chunks = vec![
                Ok(ChatCompletionResponseChunk::ToolCall(ToolCallRequest {
                    call_id: "c-hallucinated".into(),
                    name: "image_create".into(),
                    arguments: serde_json::json!({}),
                })),
                Ok(ChatCompletionResponseChunk::Done {
                    reason: ChatCompletionFinishReason::Stop,
                    usage: None,
                }),
            ];
            Ok(Box::pin(stream::iter(chunks)))
        }
    }

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<AgentResponseEvent>>);

    #[async_trait]
    impl AgentEventSink for RecordingSink {
        async fn emit(&self, event: AgentResponseEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    /// A call that fails before the toolbox accepts it must still produce the
    /// full event sequence under its `call_id`: consumers match call events by
    /// id, so a pending entry that never sees `Finished` stays pending forever.
    #[tokio::test]
    async fn unstarted_call_emits_terminal_events() {
        let sink = Arc::new(RecordingSink::default());
        let toolbox = Arc::new(ToolboxBuilder::new().build()); // no tools
        let cancellation = CancellationToken::new();

        let context: Arc<dyn ContextEngine> = Arc::new(NoopContext);
        let completion: Arc<dyn ChatCompletion> = Arc::new(HallucinatedToolCall);
        let result = react_once(
            &context,
            &[],
            &completion,
            false,
            &toolbox,
            &cancellation,
            &(sink.clone() as Arc<dyn AgentEventSink>),
        )
        .await;

        let err = result.expect_err("the turn fails with the toolbox error");
        assert!(
            matches!(err, AgentFinishKind::Failed { ref reason } if reason.contains("unknown tool")),
            "unexpected exit: {err:?}"
        );

        let events = sink.0.lock().unwrap();
        let kinds: Vec<&ToolCallEventKind> = events
            .iter()
            .filter_map(|e| match e {
                AgentResponseEvent::ToolCall { event } if event.call_id == "c-hallucinated" => {
                    Some(&event.kind)
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds.len(),
            3,
            "requested + payload + finished, got {kinds:?}"
        );
        assert!(matches!(
            &kinds[0],
            ToolCallEventKind::Requested { name, .. } if name == "image_create"
        ));
        assert!(
            matches!(&kinds[1], ToolCallEventKind::Payload { text } if text.contains("unknown tool")),
            "the failure reason rides the payload so the call's own surface shows it"
        );
        assert!(matches!(
            &kinds[2],
            ToolCallEventKind::Finished {
                status: ToolCallStatus::Error
            }
        ));
    }
}
