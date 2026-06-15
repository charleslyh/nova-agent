//! Fold persisted transcript rows into LLM request messages.

use moray_core::{
    AgentResponseEvent, ChatCompletionFinishReason, ChatCompletionRequestMessage,
    ChatCompletionResponseChunk, ToolCallEventKind, ToolCallRequest,
    ToolCallStatus,
};
use moray_session::{AgentRole, SessionEventKind};
use std::collections::HashMap;

use super::SondaSessionEventRecord;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SondaSessionSnapshot {
    pub messages: Vec<ChatCompletionRequestMessage>,
}

pub fn replay_records(records: &[SondaSessionEventRecord]) -> SondaSessionSnapshot {
    let mut messages = Vec::new();
    let mut pending_text = String::new();
    let mut pending_tools: Vec<ToolCallRequest> = Vec::new();
    let mut tool_outputs: HashMap<String, String> = HashMap::new();

    for record in records {
        match &record.event.kind {
            SessionEventKind::TurnAccepted { input } => {
                if !pending_tools.is_empty() || !pending_text.is_empty() {
                    pending_text.clear();
                    pending_tools.clear();
                }
                messages.push(input.to_user_message());
            }
            SessionEventKind::AgentResponse(chunk) => {
                if chunk.role == AgentRole::Sub {
                    continue;
                }
                fold_agent_event(
                    &chunk.event,
                    &mut messages,
                    &mut pending_text,
                    &mut pending_tools,
                    &mut tool_outputs,
                );
            }
            SessionEventKind::Reset => {
                pending_text.clear();
                pending_tools.clear();
                tool_outputs.clear();
                messages.clear();
            }
            SessionEventKind::TurnFinish => {}
        }
    }

    if !pending_tools.is_empty() || !pending_text.is_empty() {
        flush_pending_assistant(&mut messages, &mut pending_text, &mut pending_tools);
    }

    SondaSessionSnapshot { messages }
}

fn flush_pending_assistant(
    messages: &mut Vec<ChatCompletionRequestMessage>,
    pending_text: &mut String,
    pending_tools: &mut Vec<ToolCallRequest>,
) {
    if pending_text.is_empty() && pending_tools.is_empty() {
        return;
    }
    let tool_calls = if pending_tools.is_empty() {
        None
    } else {
        Some(std::mem::take(pending_tools))
    };
    messages.push(ChatCompletionRequestMessage::Assistant {
        content: std::mem::take(pending_text),
        tool_calls,
    });
}

fn fold_agent_event(
    agent_ev: &AgentResponseEvent,
    messages: &mut Vec<ChatCompletionRequestMessage>,
    pending_text: &mut String,
    pending_tools: &mut Vec<ToolCallRequest>,
    tool_outputs: &mut HashMap<String, String>,
) {
    match agent_ev {
        AgentResponseEvent::Started => {}
        AgentResponseEvent::CompletionResponse { chunk } => match chunk {
            ChatCompletionResponseChunk::TextBlock(s) => {
                pending_text.push_str(s);
            }
            ChatCompletionResponseChunk::Think(_) => {}
            ChatCompletionResponseChunk::ThinkDone => {}
            ChatCompletionResponseChunk::TextDone => {}
            ChatCompletionResponseChunk::ToolCall(tc) => {
                pending_tools.push(tc.clone());
            }
            ChatCompletionResponseChunk::Done { reason } => {
                let refusal_str = match reason {
                    ChatCompletionFinishReason::Refusal { reason } => {
                        reason.as_deref().unwrap_or("")
                    }
                    _ => "",
                };
                let mut content = std::mem::take(pending_text);
                if !refusal_str.is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(refusal_str);
                }
                let tool_calls_opt = if pending_tools.is_empty() {
                    None
                } else {
                    Some(std::mem::take(pending_tools))
                };
                if !content.is_empty() || tool_calls_opt.is_some() {
                    messages.push(ChatCompletionRequestMessage::Assistant {
                        content,
                        tool_calls: tool_calls_opt,
                    });
                }
            }
        },
        AgentResponseEvent::ToolCall { event } => match &event.kind {
            ToolCallEventKind::Requested { .. } => {}
            ToolCallEventKind::Extra { .. } => {}
            ToolCallEventKind::Started => {}
            ToolCallEventKind::Payload { text } => {
                let acc = tool_outputs.entry(event.call_id.clone()).or_default();
                acc.push_str(text);
            }
            ToolCallEventKind::Finished { status } => {
                flush_pending_assistant(messages, pending_text, pending_tools);
                let call_id = event.call_id.clone();
                let content = tool_outputs.remove(&call_id).unwrap_or_default();
                if *status == ToolCallStatus::Canceled {
                    return;
                }
                let content = if *status == ToolCallStatus::Error && content.is_empty() {
                    "tool call failed".to_string()
                } else {
                    content
                };
                messages.push(ChatCompletionRequestMessage::Tool {
                    content,
                    call_id,
                });
            }
        },
        AgentResponseEvent::Finished { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moray_core::{AgentFinishKind, ToolCallEvent, ToolCallStatus};
    use moray_session::{AgentRole, SessionAgentResponse, SessionEvent, SessionEventKind, TurnInput};
    use serde_json::{json, Value};

    fn record(seq: u64, kind: SessionEventKind) -> SondaSessionEventRecord {
        SondaSessionEventRecord {
            seq,
            event: SessionEvent {
                session_id: "test-session".into(),
                ts: 0,
                kind,
            },
        }
    }

    fn user(seq: u64, c: &str) -> SondaSessionEventRecord {
        record(
            seq,
            SessionEventKind::TurnAccepted {
                input: TurnInput {
                    text: c.into(),
                    resources: Vec::new(),
                },
            },
        )
    }

    fn agent(
        seq: u64,
        agent_id: &str,
        role: AgentRole,
        event: AgentResponseEvent,
    ) -> SondaSessionEventRecord {
        record(
            seq,
            SessionEventKind::AgentResponse(SessionAgentResponse {
                agent_id: agent_id.into(),
                role,
                event,
            }),
        )
    }

    fn tb(s: &str) -> AgentResponseEvent {
        AgentResponseEvent::CompletionResponse {
            chunk: ChatCompletionResponseChunk::TextBlock(s.into()),
        }
    }

    fn td() -> AgentResponseEvent {
        AgentResponseEvent::CompletionResponse {
            chunk: ChatCompletionResponseChunk::TextDone,
        }
    }

    fn done_stop() -> AgentResponseEvent {
        AgentResponseEvent::CompletionResponse {
            chunk: ChatCompletionResponseChunk::Done {
                reason: ChatCompletionFinishReason::Stop,
            },
        }
    }

    fn tc(call_id: &str, name: &str, args: Value) -> AgentResponseEvent {
        AgentResponseEvent::CompletionResponse {
            chunk: ChatCompletionResponseChunk::ToolCall(ToolCallRequest {
                call_id: call_id.into(),
                name: name.into(),
                arguments: args,
            }),
        }
    }

    fn tcf(call_id: &str, content: &str) -> AgentResponseEvent {
        AgentResponseEvent::ToolCall {
            event: ToolCallEvent::payload(call_id.into(), content.into()),
        }
    }

    fn tff(call_id: &str, status: ToolCallStatus) -> AgentResponseEvent {
        AgentResponseEvent::ToolCall {
            event: ToolCallEvent::finished(call_id.into(), status),
        }
    }

    fn finished(kind: AgentFinishKind) -> AgentResponseEvent {
        AgentResponseEvent::Finished { kind }
    }

    fn assert_user_message(messages: &[ChatCompletionRequestMessage], index: usize, content: &str) {
        let got = messages.get(index);
        assert!(
            matches!(
                got,
                Some(ChatCompletionRequestMessage::User { content: c }) if c == content
            ),
            "expected User {{ content: {content:?} }} at index {index}, got {got:?}"
        );
    }

    fn assert_assistant_text(messages: &[ChatCompletionRequestMessage], index: usize, text: &str) {
        let got = messages.get(index);
        assert!(
            matches!(
                got,
                Some(ChatCompletionRequestMessage::Assistant {
                    content,
                    tool_calls: None
                }) if content == text
            ),
            "expected Assistant text {text:?} at index {index}, got {got:?}"
        );
    }

    fn assert_assistant_tool_calls(
        messages: &[ChatCompletionRequestMessage],
        index: usize,
        call_id: &str,
        name: &str,
    ) {
        let got = messages.get(index);
        assert!(
            matches!(
                got,
                Some(ChatCompletionRequestMessage::Assistant {
                    tool_calls: Some(calls),
                    ..
                }) if calls.len() == 1
                    && calls[0].call_id == call_id
                    && calls[0].name == name
            ),
            "expected Assistant tool_calls [{call_id}/{name}] at index {index}, got {got:?}"
        );
    }

    fn assert_tool_message(
        messages: &[ChatCompletionRequestMessage],
        index: usize,
        call_id: &str,
        content: &str,
    ) {
        let got = messages.get(index);
        assert!(
            matches!(
                got,
                Some(ChatCompletionRequestMessage::Tool {
                    call_id: id,
                    content: c
                }) if id == call_id && c == content
            ),
            "expected Tool {{ call_id: {call_id}, content: {content:?} }} at index {index}, got {got:?}"
        );
    }

    #[test]
    fn closed_turn_replay_ignores_trailing_finished_event() {
        let records = vec![
            user(1, "hi"),
            agent(2, "leader", AgentRole::Leader, tb("partial")),
            agent(3, "leader", AgentRole::Leader, td()),
            agent(4, "leader", AgentRole::Leader, done_stop()),
            agent(5, "leader", AgentRole::Leader, finished(AgentFinishKind::Succeeded)),
        ];
        let s = replay_records(&records);
        assert_eq!(s.messages.len(), 2);
        assert_user_message(&s.messages, 0, "hi");
        assert_assistant_text(&s.messages, 1, "partial");
    }

    #[test]
    fn replay_session_closes_assistant_at_done() {
        let records = vec![
            user(1, "hi"),
            agent(2, "leader", AgentRole::Leader, tb("hel")),
            agent(3, "leader", AgentRole::Leader, td()),
            agent(4, "leader", AgentRole::Leader, done_stop()),
        ];
        let s = replay_records(&records);
        assert_eq!(s.messages.len(), 2);
        assert_user_message(&s.messages, 0, "hi");
        assert_assistant_text(&s.messages, 1, "hel");
    }

    #[test]
    fn new_user_aborts_partial_assistant() {
        let records = vec![user(1, "hi"), agent(2, "leader", AgentRole::Leader, tb("hel")), user(3, "next")];
        let s = replay_records(&records);
        assert_eq!(s.messages.len(), 2);
        assert_user_message(&s.messages, 0, "hi");
        assert_user_message(&s.messages, 1, "next");
    }

    #[test]
    fn tool_call_canceled_skips_tool_message() {
        let records = vec![
            user(1, "hi"),
            agent(2, "leader", AgentRole::Leader, td()),
            agent(3, "leader", AgentRole::Leader, tc("c1", "echo", json!({}))),
            agent(4, "leader", AgentRole::Leader, done_stop()),
            agent(5, "leader", AgentRole::Leader, tff("c1", ToolCallStatus::Canceled)),
        ];
        let s = replay_records(&records);
        assert_eq!(s.messages.len(), 2);
        assert_user_message(&s.messages, 0, "hi");
        assert_assistant_tool_calls(&s.messages, 1, "c1", "echo");
        assert!(!s.messages.iter().any(|m| matches!(
            m,
            ChatCompletionRequestMessage::Tool { .. }
        )));
    }

    #[test]
    fn tool_call_finished_appends_tool_message() {
        let records = vec![
            user(1, "hi"),
            agent(2, "leader", AgentRole::Leader, td()),
            agent(3, "leader", AgentRole::Leader, tc("c1", "echo", json!({}))),
            agent(4, "leader", AgentRole::Leader, done_stop()),
            agent(5, "leader", AgentRole::Leader, tcf("c1", "ok")),
            agent(6, "leader", AgentRole::Leader, tff("c1", ToolCallStatus::Success)),
        ];
        let s = replay_records(&records);
        assert_eq!(s.messages.len(), 3);
        assert_user_message(&s.messages, 0, "hi");
        assert_assistant_tool_calls(&s.messages, 1, "c1", "echo");
        assert_tool_message(&s.messages, 2, "c1", "ok");
    }
}
