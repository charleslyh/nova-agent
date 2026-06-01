//! QQ outbound: session event → plain-text IM replies.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use moray_core::{parse_tool_call_args, AgentFinishKind, AgentResponseEvent, ToolCallEvent};
use moray_session::{SessionEvent, SessionEventKind};
use tokio::sync::RwLock;

use crate::channels::qq::approval::format_approval_prompt;
use crate::channels::qq::reply::QqReplyRoute;

use super::channel::QQChannel;
use crate::channels::render::{chunk_text, collapse_blank_lines, friendly_tool_label, strip_think_tags};

const TOOL_PLACEHOLDER_OPEN: char = '\u{0001}';
const TOOL_PLACEHOLDER_CLOSE: char = '\u{0002}';

fn tool_placeholder(call_id: &str) -> String {
    format!("{TOOL_PLACEHOLDER_OPEN}TOOL:{call_id}{TOOL_PLACEHOLDER_CLOSE}")
}

#[derive(Debug, Clone)]
struct PendingToolAuth {
    tool_name: String,
    display_name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Default)]
struct QqTurnState {
    body: String,
    tool_displays: HashMap<String, String>,
    pending_auth: HashMap<String, PendingToolAuth>,
    auth_prompted: HashSet<String>,
    think_tag_buf: String,
}

/// Per-session outbound state for one QQ channel instance.
pub struct QqSessionOutbound {
    channel: Arc<QQChannel>,
    reply: Arc<RwLock<Option<QqReplyRoute>>>,
    state: QqTurnState,
}

impl QqSessionOutbound {
    pub fn new(channel: Arc<QQChannel>) -> Self {
        Self {
            channel,
            reply: Arc::new(RwLock::new(None)),
            state: QqTurnState::default(),
        }
    }

    pub async fn set_reply_route(&self, route: QqReplyRoute) {
        *self.reply.write().await = Some(route);
    }

    pub async fn on_session_event(&mut self, event: &SessionEvent) {
        match &event.kind {
            SessionEventKind::AgentResponse { agent } => self.on_agent_event(agent).await,
            SessionEventKind::TurnFinish => self.on_turn_finish().await,
            SessionEventKind::Reset => {
                self.state = QqTurnState::default();
            }
            SessionEventKind::TurnAccepted { .. } => {}
        }
    }

    async fn on_agent_event(&mut self, ev: &AgentResponseEvent) {
        match ev {
            AgentResponseEvent::CompletionResponse { chunk } => {
                if let Some(text) = chunk_text(chunk) {
                    self.handle_chunk(&text).await;
                }
            }
            AgentResponseEvent::ToolCall { event } => match event {
                ToolCallEvent::Requested { content } => {
                    let call_id = content.call_id.clone();
                    let display = friendly_tool_label(&content.name, &content.name);
                    self.state.pending_auth.insert(
                        call_id.clone(),
                        PendingToolAuth {
                            tool_name: content.name.clone(),
                            display_name: display.clone(),
                            arguments: parse_tool_call_args(&content.arguments),
                        },
                    );
                    self.state.tool_displays.insert(call_id, display);
                }
                ToolCallEvent::Started { call_id } => {
                    self.insert_tool_placeholder(call_id);
                }
                ToolCallEvent::Completed { .. } => {}
                ToolCallEvent::Custom { call_id, .. } => {
                    self.handle_tool_auth(call_id).await;
                }
            },
            AgentResponseEvent::Finished { kind } => {
                if let AgentFinishKind::Failed { reason } = kind {
                    self.handle_chunk(&format!("\n\nError: {reason}")).await;
                }
            }
            AgentResponseEvent::Started => {}
        }
    }

    async fn on_turn_finish(&mut self) {
        let body = self.render_plain_body();
        if !body.is_empty() {
            if let Some(reply) = self.reply.read().await.clone() {
                let _ = self.channel.deliver_plain_text(&reply, &body).await;
            }
        }
        self.state = QqTurnState::default();
    }

    fn render_plain_body(&self) -> String {
        let mut out = self.state.body.clone();
        for call_id in self.state.tool_displays.keys() {
            let ph = tool_placeholder(call_id);
            for pattern in [
                format!("\n{ph}\n"),
                format!("{ph}\n"),
                format!("\n{ph}"),
                ph,
            ] {
                out = out.replace(&pattern, "\n");
            }
        }
        collapse_blank_lines(out.trim())
    }

    async fn handle_chunk(&mut self, content: &str) {
        let stripped = strip_think_tags(&mut self.state.think_tag_buf, content);
        if stripped.is_empty() {
            return;
        }
        self.state.body.push_str(&stripped);
    }

    fn insert_tool_placeholder(&mut self, call_id: &str) {
        if !self.state.body.is_empty() && !self.state.body.ends_with('\n') {
            self.state.body.push('\n');
        }
        self.state.body.push_str(&tool_placeholder(call_id));
        self.state.body.push('\n');
    }

    async fn handle_tool_auth(&mut self, call_id: &str) {
        if self.state.auth_prompted.contains(call_id) {
            return;
        }
        let Some(pending) = self.state.pending_auth.get(call_id).cloned() else {
            tracing::warn!(call_id, "QQ tool auth Custom without prior Requested event");
            return;
        };
        self.state.auth_prompted.insert(call_id.to_string());

        if let Some(reply) = self.reply.read().await.clone() {
            let message = format_approval_prompt(
                &pending.tool_name,
                &pending.display_name,
                call_id,
                &pending.arguments,
                220,
            );
            if let Err(e) = self.channel.deliver_plain_text(&reply, &message).await {
                tracing::warn!(call_id, error = %e, "QQ send approval prompt failed");
            }
        }

    }
}
