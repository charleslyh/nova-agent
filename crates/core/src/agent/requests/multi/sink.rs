use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::agent::requests::single::AgentEventSink;
use crate::agent::types::{AgentFinishKind, AgentResponseEvent};
use crate::completion::ChatCompletionResponseChunk;
use crate::toolbox::ToolCallEventKind;

use super::formatter::{AgentResultFormatter, SubAgentRunData, ToolCallRecord};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AgentRole {
    Leader,
    Sub,
}

/// Multi-agent event envelope. `data` stays nested in session events to avoid
/// clashing with the outer `type` tag; use [`MultiAgentResponseEvent::flatten`]
/// when a single-level JSON object is needed.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MultiAgentResponseEvent {
    pub agent_id: String,
    pub role: AgentRole,
    /// For `Sub` events: the leader's tool `call_id` that triggered this sub-agent run.
    /// Enables the consumer to correlate concurrent calls to the same `agent_id`.
    #[cfg_attr(
        feature = "serde",
        serde(skip_serializing_if = "Option::is_none", default)
    )]
    pub call_id: Option<String>,
    pub data: AgentResponseEvent,
}

impl MultiAgentResponseEvent {
    /// Flatten `agent_id`, `role`, `call_id`, and response fields into one JSON object.
    #[cfg(feature = "serde")]
    pub fn flatten(&self) -> Result<serde_json::Value, serde_json::Error> {
        let mut value = serde_json::to_value(&self.data)?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "agent_id".into(),
                serde_json::Value::String(self.agent_id.clone()),
            );
            obj.insert("role".into(), serde_json::to_value(self.role)?);
            if let Some(call_id) = &self.call_id {
                obj.insert("call_id".into(), serde_json::Value::String(call_id.clone()));
            }
        }
        Ok(value)
    }
}

/// Push sink for multi-agent [`MultiAgentResponseEvent`] values.
/// Emit failures are ignored, matching channel `send` best-effort semantics.
#[async_trait]
pub trait MultiAgentEventSink: Send + Sync {
    async fn emit(&self, event: MultiAgentResponseEvent);
}

/// Tokio channel adapter for [`MultiAgentEventSink`].
pub struct ChannelMultiAgentEventSink {
    tx: mpsc::Sender<MultiAgentResponseEvent>,
}

impl ChannelMultiAgentEventSink {
    pub fn new(tx: mpsc::Sender<MultiAgentResponseEvent>) -> Self {
        Self { tx }
    }

    pub fn sender(&self) -> &mpsc::Sender<MultiAgentResponseEvent> {
        &self.tx
    }
}

#[async_trait]
impl MultiAgentEventSink for ChannelMultiAgentEventSink {
    async fn emit(&self, event: MultiAgentResponseEvent) {
        let _ = self.tx.send(event).await;
    }
}

struct BridgingAgentEventSink {
    multi: Arc<dyn MultiAgentEventSink>,
    agent_id: String,
    role: AgentRole,
    call_id: Option<String>,
}

#[async_trait]
impl AgentEventSink for BridgingAgentEventSink {
    async fn emit(&self, data: AgentResponseEvent) {
        self.multi
            .emit(MultiAgentResponseEvent {
                agent_id: self.agent_id.clone(),
                role: self.role,
                call_id: self.call_id.clone(),
                data,
            })
            .await;
    }
}

/// Wraps a [`MultiAgentEventSink`] as a single-agent sink for one agent identity.
pub(crate) fn bridge_agent_events(
    multi: Arc<dyn MultiAgentEventSink>,
    agent_id: String,
    role: AgentRole,
) -> Arc<dyn AgentEventSink> {
    Arc::new(BridgingAgentEventSink {
        multi,
        agent_id,
        role,
        call_id: None,
    })
}

/// Forwards events and collects the sub-agent's final text for tool return values.
pub(crate) struct CollectingAgentEventSink {
    multi: Arc<dyn MultiAgentEventSink>,
    agent_id: String,
    role: AgentRole,
    call_id: Option<String>,
    reducer: Mutex<SubAgentResultReducer>,
}

impl CollectingAgentEventSink {
    pub(crate) fn new(
        multi: Arc<dyn MultiAgentEventSink>,
        agent_id: String,
        role: AgentRole,
        call_id: Option<String>,
        formatter: Option<Arc<dyn AgentResultFormatter>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            multi,
            agent_id,
            role,
            call_id,
            reducer: Mutex::new(SubAgentResultReducer::new(formatter)),
        })
    }

    pub(crate) fn finalize(&self) -> std::result::Result<String, String> {
        let mut guard = self
            .reducer
            .lock()
            .expect("sub-agent reducer lock poisoned");
        let formatter = guard.formatter.clone();
        std::mem::replace(&mut *guard, SubAgentResultReducer::new(formatter)).into_result()
    }
}

#[async_trait]
impl AgentEventSink for CollectingAgentEventSink {
    async fn emit(&self, data: AgentResponseEvent) {
        self.reducer
            .lock()
            .expect("sub-agent reducer lock poisoned")
            .on_event(&data);
        self.multi
            .emit(MultiAgentResponseEvent {
                agent_id: self.agent_id.clone(),
                role: self.role,
                call_id: self.call_id.clone(),
                data,
            })
            .await;
    }
}

pub(crate) struct SubAgentResultReducer {
    pending_text: String,
    final_text: Option<String>,
    exit_error: Option<String>,
    /// Optional formatter for structuring the final result from intermediate data.
    formatter: Option<Arc<dyn AgentResultFormatter>>,
    /// Completed tool call records (pushed once a call reaches `Finished`).
    tool_call_records: Vec<ToolCallRecord>,
    /// In-flight tool calls being aggregated (keyed by call_id).
    pending_calls: HashMap<String, ToolCallRecord>,
}

impl SubAgentResultReducer {
    pub(crate) fn new(formatter: Option<Arc<dyn AgentResultFormatter>>) -> Self {
        Self {
            pending_text: String::new(),
            final_text: None,
            exit_error: None,
            formatter,
            tool_call_records: Vec::new(),
            pending_calls: HashMap::new(),
        }
    }

    pub(crate) fn into_result(mut self) -> std::result::Result<String, String> {
        if let Some(err) = self.exit_error {
            return Err(err);
        }

        // Flush any pending (unfinished) calls into records so the formatter sees them.
        for (_, record) in self.pending_calls.drain() {
            self.tool_call_records.push(record);
        }

        // If a formatter is present, use it to produce the final result from intermediate data.
        if let Some(formatter) = &self.formatter {
            let run_data = SubAgentRunData {
                tool_calls: std::mem::take(&mut self.tool_call_records),
            };
            return formatter.format(&run_data);
        }

        Ok(self.final_text.unwrap_or_default())
    }

    pub(crate) fn on_event(&mut self, ev: &AgentResponseEvent) {
        match ev {
            AgentResponseEvent::Started => {
                self.pending_text.clear();
            }
            AgentResponseEvent::CompletionResponse { chunk } => match chunk {
                ChatCompletionResponseChunk::TextBlock(s) => {
                    self.pending_text.push_str(s);
                }
                ChatCompletionResponseChunk::ToolCall(_) => {
                    self.pending_text.clear();
                }
                ChatCompletionResponseChunk::Done { .. } => {
                    self.capture_pending_if_nonempty();
                }
                _ => {}
            },
            AgentResponseEvent::ToolCall { event } => {
                self.on_tool_call_event(event);
            }
            AgentResponseEvent::Finished { kind } => match kind {
                AgentFinishKind::Succeeded | AgentFinishKind::RoundLimitReached => {
                    self.capture_pending_if_nonempty();
                }
                AgentFinishKind::Canceled => {
                    self.capture_pending_if_nonempty();
                    if self.final_text.is_none() {
                        self.exit_error = Some("sub agent run canceled".into());
                    }
                }
                AgentFinishKind::Refused { reason } => {
                    self.exit_error = Some(
                        reason
                            .clone()
                            .filter(|r| !r.trim().is_empty())
                            .unwrap_or_else(|| "sub agent refused".into()),
                    );
                }
                AgentFinishKind::Failed { reason } => {
                    self.exit_error = Some(format!("sub agent failed: {reason}"));
                }
            },
        }
    }

    fn on_tool_call_event(&mut self, event: &crate::toolbox::ToolCallEvent) {
        let call_id = &event.call_id;
        match &event.kind {
            ToolCallEventKind::Requested { name, arguments } => {
                let record = ToolCallRecord {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    arguments: arguments.clone(),
                    content: String::new(),
                    status: None,
                };
                self.pending_calls.insert(call_id.clone(), record);
            }
            ToolCallEventKind::Payload { text } => {
                if let Some(record) = self.pending_calls.get_mut(call_id) {
                    record.content.push_str(text);
                }
            }
            ToolCallEventKind::Finished { status } => {
                if let Some(mut record) = self.pending_calls.remove(call_id) {
                    record.status = Some(status.clone());
                    self.tool_call_records.push(record);
                }
            }
            // Started / Extra events are not captured in the record.
            _ => {}
        }
    }

    fn capture_pending_if_nonempty(&mut self) {
        let text = std::mem::take(&mut self.pending_text);
        if !text.trim().is_empty() {
            self.final_text = Some(text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatCompletionFinishReason, ToolCallRequest};
    use serde_json::json;

    fn text_block(s: &str) -> AgentResponseEvent {
        AgentResponseEvent::CompletionResponse {
            chunk: ChatCompletionResponseChunk::TextBlock(s.into()),
        }
    }

    fn completion_done() -> AgentResponseEvent {
        AgentResponseEvent::CompletionResponse {
            chunk: ChatCompletionResponseChunk::Done {
                reason: ChatCompletionFinishReason::Stop,
                usage: None,
            },
        }
    }

    fn tool_call_chunk() -> AgentResponseEvent {
        AgentResponseEvent::CompletionResponse {
            chunk: ChatCompletionResponseChunk::ToolCall(ToolCallRequest {
                call_id: "c1".into(),
                name: "image_create".into(),
                arguments: json!({}),
            }),
        }
    }

    #[test]
    fn sub_reducer_keeps_last_nonempty_round_after_tool_call() {
        let mut reducer = SubAgentResultReducer::new(None);
        reducer.on_event(&text_block("partial "));
        reducer.on_event(&tool_call_chunk());
        reducer.on_event(&completion_done());
        reducer.on_event(&text_block("final summary"));
        reducer.on_event(&completion_done());
        assert_eq!(reducer.into_result(), Ok("final summary".into()));
    }

    #[test]
    fn sub_reducer_preserves_partial_text_on_cancel() {
        let mut reducer = SubAgentResultReducer::new(None);
        reducer.on_event(&text_block("partial result"));
        reducer.on_event(&AgentResponseEvent::Finished {
            kind: AgentFinishKind::Canceled,
        });
        assert_eq!(reducer.into_result(), Ok("partial result".into()));
    }

    #[test]
    fn sub_reducer_cancel_without_text_returns_error() {
        let mut reducer = SubAgentResultReducer::new(None);
        reducer.on_event(&AgentResponseEvent::Finished {
            kind: AgentFinishKind::Canceled,
        });
        assert_eq!(
            reducer.into_result(),
            Err("sub agent run canceled".into())
        );
    }

    // --- Formatter tests ---

    use crate::toolbox::{ToolCallEvent, ToolCallEventKind};
    use crate::types::ToolCallStatus;

    struct MockFormatter;

    impl AgentResultFormatter for MockFormatter {
        fn format(&self, data: &SubAgentRunData) -> std::result::Result<String, String> {
            // Produce a simple summary of all tool calls.
            let summary: Vec<String> = data
                .tool_calls
                .iter()
                .map(|r| format!("{}({})->{}", r.name, r.arguments, r.content))
                .collect();
            Ok(summary.join("; "))
        }
    }

    fn tool_call_event(call_id: &str, kind: ToolCallEventKind) -> AgentResponseEvent {
        AgentResponseEvent::ToolCall {
            event: ToolCallEvent {
                call_id: call_id.to_string(),
                kind,
            },
        }
    }

    #[test]
    fn sub_reducer_formatter_produces_result_from_tool_calls() {
        let formatter: Arc<dyn AgentResultFormatter> = Arc::new(MockFormatter);
        let mut reducer = SubAgentResultReducer::new(Some(formatter));

        // Simulate a tool call lifecycle: Requested → Payload → Finished
        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Requested {
                name: "read_file".into(),
                arguments: json!({"path": "/tmp/a.txt"}),
            },
        ));
        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Payload {
                text: "file content".into(),
            },
        ));
        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Finished {
                status: ToolCallStatus::Success,
            },
        ));

        // Also emit some text (should be ignored when formatter is present)
        reducer.on_event(&text_block("some final text"));
        reducer.on_event(&completion_done());
        reducer.on_event(&AgentResponseEvent::Finished {
            kind: AgentFinishKind::Succeeded,
        });

        let result = reducer.into_result();
        assert_eq!(
            result,
            Ok("read_file({\"path\":\"/tmp/a.txt\"})->file content".into())
        );
    }

    #[test]
    fn sub_reducer_without_formatter_returns_final_text() {
        let mut reducer = SubAgentResultReducer::new(None);

        // Simulate a tool call
        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Requested {
                name: "read_file".into(),
                arguments: json!({}),
            },
        ));
        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Finished {
                status: ToolCallStatus::Success,
            },
        ));

        // Emit final text
        reducer.on_event(&text_block("the result"));
        reducer.on_event(&completion_done());
        reducer.on_event(&AgentResponseEvent::Finished {
            kind: AgentFinishKind::Succeeded,
        });

        assert_eq!(reducer.into_result(), Ok("the result".into()));
    }

    #[test]
    fn sub_reducer_collects_multiple_tool_calls() {
        let formatter: Arc<dyn AgentResultFormatter> = Arc::new(MockFormatter);
        let mut reducer = SubAgentResultReducer::new(Some(formatter));

        // First tool call
        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Requested {
                name: "tool_a".into(),
                arguments: json!("arg1"),
            },
        ));
        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Payload {
                text: "result1".into(),
            },
        ));
        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Finished {
                status: ToolCallStatus::Success,
            },
        ));

        // Second tool call
        reducer.on_event(&tool_call_event(
            "c2",
            ToolCallEventKind::Requested {
                name: "tool_b".into(),
                arguments: json!("arg2"),
            },
        ));
        reducer.on_event(&tool_call_event(
            "c2",
            ToolCallEventKind::Payload {
                text: "result2".into(),
            },
        ));
        reducer.on_event(&tool_call_event(
            "c2",
            ToolCallEventKind::Finished {
                status: ToolCallStatus::Success,
            },
        ));

        reducer.on_event(&AgentResponseEvent::Finished {
            kind: AgentFinishKind::Succeeded,
        });

        let result = reducer.into_result();
        assert_eq!(
            result,
            Ok("tool_a(\"arg1\")->result1; tool_b(\"arg2\")->result2".into())
        );
    }

    #[test]
    fn sub_reducer_error_takes_precedence_over_formatter() {
        let formatter: Arc<dyn AgentResultFormatter> = Arc::new(MockFormatter);
        let mut reducer = SubAgentResultReducer::new(Some(formatter));

        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Requested {
                name: "some_tool".into(),
                arguments: json!({}),
            },
        ));
        reducer.on_event(&AgentResponseEvent::Finished {
            kind: AgentFinishKind::Failed {
                reason: "timeout".into(),
            },
        });

        assert_eq!(
            reducer.into_result(),
            Err("sub agent failed: timeout".into())
        );
    }

    #[test]
    fn sub_reducer_pending_calls_flushed_into_records() {
        let formatter: Arc<dyn AgentResultFormatter> = Arc::new(MockFormatter);
        let mut reducer = SubAgentResultReducer::new(Some(formatter));

        // A tool call that never finishes (only Requested + Payload, no Finished)
        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Requested {
                name: "dangling_tool".into(),
                arguments: json!(null),
            },
        ));
        reducer.on_event(&tool_call_event(
            "c1",
            ToolCallEventKind::Payload {
                text: "partial".into(),
            },
        ));

        reducer.on_event(&AgentResponseEvent::Finished {
            kind: AgentFinishKind::Succeeded,
        });

        let result = reducer.into_result();
        // Even without Finished, the pending call is flushed to records
        assert_eq!(
            result,
            Ok("dangling_tool(null)->partial".into())
        );
    }
}
