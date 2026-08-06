use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::agent::requests::single::AgentEventSink;
use crate::agent::types::{AgentFinishKind, AgentResponseEvent};
use crate::completion::ChatCompletionResponseChunk;

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
    pub data: AgentResponseEvent,
}

impl MultiAgentResponseEvent {
    /// Flatten `agent_id`, `role`, and response fields into one JSON object.
    #[cfg(feature = "serde")]
    pub fn flatten(&self) -> Result<serde_json::Value, serde_json::Error> {
        let mut value = serde_json::to_value(&self.data)?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "agent_id".into(),
                serde_json::Value::String(self.agent_id.clone()),
            );
            obj.insert("role".into(), serde_json::to_value(self.role)?);
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
}

#[async_trait]
impl AgentEventSink for BridgingAgentEventSink {
    async fn emit(&self, data: AgentResponseEvent) {
        self.multi
            .emit(MultiAgentResponseEvent {
                agent_id: self.agent_id.clone(),
                role: self.role,
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
    })
}

/// Forwards events and collects the sub-agent's final text for tool return values.
pub(crate) struct CollectingAgentEventSink {
    multi: Arc<dyn MultiAgentEventSink>,
    agent_id: String,
    role: AgentRole,
    reducer: Mutex<SubAgentResultReducer>,
}

impl CollectingAgentEventSink {
    pub(crate) fn new(
        multi: Arc<dyn MultiAgentEventSink>,
        agent_id: String,
        role: AgentRole,
    ) -> Arc<Self> {
        Arc::new(Self {
            multi,
            agent_id,
            role,
            reducer: Mutex::new(SubAgentResultReducer::new()),
        })
    }

    pub(crate) fn finalize(&self) -> std::result::Result<String, String> {
        let mut guard = self
            .reducer
            .lock()
            .expect("sub-agent reducer lock poisoned");
        std::mem::replace(&mut *guard, SubAgentResultReducer::new()).into_result()
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
                data,
            })
            .await;
    }
}

pub(crate) struct SubAgentResultReducer {
    pending_text: String,
    final_text: Option<String>,
    exit_error: Option<String>,
}

impl SubAgentResultReducer {
    pub(crate) fn new() -> Self {
        Self {
            pending_text: String::new(),
            final_text: None,
            exit_error: None,
        }
    }

    pub(crate) fn into_result(self) -> std::result::Result<String, String> {
        if let Some(err) = self.exit_error {
            return Err(err);
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
            AgentResponseEvent::ToolCall { .. } => {}
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
        let mut reducer = SubAgentResultReducer::new();
        reducer.on_event(&text_block("partial "));
        reducer.on_event(&tool_call_chunk());
        reducer.on_event(&completion_done());
        reducer.on_event(&text_block("final summary"));
        reducer.on_event(&completion_done());
        assert_eq!(reducer.into_result(), Ok("final summary".into()));
    }

    #[test]
    fn sub_reducer_preserves_partial_text_on_cancel() {
        let mut reducer = SubAgentResultReducer::new();
        reducer.on_event(&text_block("partial result"));
        reducer.on_event(&AgentResponseEvent::Finished {
            kind: AgentFinishKind::Canceled,
        });
        assert_eq!(reducer.into_result(), Ok("partial result".into()));
    }

    #[test]
    fn sub_reducer_cancel_without_text_returns_error() {
        let mut reducer = SubAgentResultReducer::new();
        reducer.on_event(&AgentResponseEvent::Finished {
            kind: AgentFinishKind::Canceled,
        });
        assert_eq!(
            reducer.into_result(),
            Err("sub agent run canceled".into())
        );
    }
}
