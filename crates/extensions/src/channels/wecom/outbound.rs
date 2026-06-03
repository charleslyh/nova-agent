//! WeCom outbound: streaming draft cards with inline tool status markers.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use moray_core::{
    parse_tool_call_args, AgentFinishKind, AgentResponseEvent, ToolCallEventKind,
    ToolCallStatus,
};
use moray_session::{SessionEvent, SessionEventKind};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::channels::wecom::reply::WeComReplyRoute;
use crate::channels::render::{chunk_text, extract_output_preview, friendly_tool_label, strip_think_tags};
use super::channel::WeComChannel;

const TOOL_PLACEHOLDER_OPEN: char = '\u{0001}';
const TOOL_PLACEHOLDER_CLOSE: char = '\u{0002}';
/// 流式 draft 最短推送间隔（企微 stream 为全量替换，过于频繁会加重 WS 压力）。
const DRAFT_PUSH_MIN_INTERVAL_MS: u128 = 1000;
/// 距上次推送新增至少这么多字符时允许提前推送（避免长句只在 turn 结束才露面）。
const DRAFT_PUSH_MIN_CHARS: usize = 200;

fn draft_push_throttled(
    last_push_at: Option<Instant>,
    last_pushed_len: usize,
    current_len: usize,
    immediate: bool,
) -> bool {
    if immediate {
        return false;
    }
    let Some(last_push_at) = last_push_at else {
        // 首次也等字符阈值，否则走 debounce，避免几个字节就触发 WS 推送。
        return current_len < DRAFT_PUSH_MIN_CHARS;
    };
    last_push_at.elapsed().as_millis() < DRAFT_PUSH_MIN_INTERVAL_MS
        && current_len.saturating_sub(last_pushed_len) < DRAFT_PUSH_MIN_CHARS
}

fn tool_placeholder(call_id: &str) -> String {
    format!("{TOOL_PLACEHOLDER_OPEN}TOOL:{call_id}{TOOL_PLACEHOLDER_CLOSE}")
}

#[derive(Debug, Clone)]
enum ToolSlot {
    Running { display: String },
    Completed {
        display: String,
        success: bool,
        preview: String,
    },
}

#[derive(Debug, Clone)]
struct PendingToolAuth {
    tool_name: String,
    display_name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Default)]
struct WeComTurnState {
    body: String,
    tools: HashMap<String, ToolSlot>,
    tool_outputs: HashMap<String, String>,
    tool_displays: HashMap<String, String>,
    pending_auth: HashMap<String, PendingToolAuth>,
    auth_prompted: HashSet<String>,
    think_tag_buf: String,
    draft_msg_id: Option<String>,
    /// 最后一次工具事件 push 后 `render_draft_body` 的字节长度；其后追加的 chunk 视为最终回答。
    intermediate_render_len: usize,
    last_draft_push_at: Option<Instant>,
    last_pushed_body_len: usize,
    draft_push_pending: bool,
    debounce_cancel: Option<CancellationToken>,
}

fn render_tool_slot(slot: &ToolSlot) -> String {
    match slot {
        ToolSlot::Running { display } => format!("[⏳ {display}]"),
        ToolSlot::Completed {
            display,
            success: true,
            preview,
        } => {
            if preview.is_empty() {
                format!("[✅ {display}]")
            } else {
                format!("[✅ {display}]\n{preview}")
            }
        }
        ToolSlot::Completed {
            display,
            success: false,
            preview,
        } => {
            if preview.is_empty() {
                format!("[❌ {display}]")
            } else {
                format!("[❌ {display}]\n{preview}")
            }
        }
    }
}

fn render_draft_body(state: &WeComTurnState) -> String {
    let mut out = state.body.clone();
    for (call_id, slot) in &state.tools {
        let placeholder = tool_placeholder(call_id);
        let rendered = render_tool_slot(slot);
        out = out.replace(&placeholder, &rendered);
    }
    out
}

/// 按工具完成后的渲染边界拆分 intermediate / final（对齐 moray 的 final_text 剥离语义）。
fn split_body_at_render_boundary(body_render: &str, intermediate_len: usize) -> (String, String) {
    let trimmed = body_render.trim();
    if trimmed.is_empty() {
        return (String::new(), String::new());
    }
    if intermediate_len == 0 {
        let has_tool = trimmed.contains("[✅")
            || trimmed.contains("[❌")
            || trimmed.contains("[⏳");
        if has_tool {
            return (trimmed.to_string(), String::new());
        }
        return (String::new(), trimmed.to_string());
    }
    if intermediate_len >= body_render.len() {
        return (body_render.trim_end().to_string(), String::new());
    }
    let think = body_render[..intermediate_len].trim_end().to_string();
    let final_body = body_render[intermediate_len..].trim_start().to_string();
    (think, final_body)
}

fn build_finalize_payload(body_render: &str, intermediate_len: usize) -> String {
    let (intermediate, final_body) = split_body_at_render_boundary(body_render, intermediate_len);
    if intermediate.is_empty() {
        if final_body.is_empty() {
            return String::new();
        }
        // 无工具：闭合思考块并保留正文，避免 finalize 后企微把流式「思考」气泡换成纯文本。
        return format!(
            "<think>\n{final_body}\n</think>\n\n{final_body}"
        );
    }
    if final_body.is_empty() {
        format!("<think>\n{intermediate}\n</think>")
    } else {
        format!(
            "<think>\n{intermediate}\n</think>\n\n{final_body}"
        )
    }
}

fn draft_update_display(body_render: &str) -> String {
    if body_render.is_empty() {
        String::new()
    } else {
        format!("<think>\n{body_render}")
    }
}

pub struct WeComSessionOutbound {
    channel: Arc<WeComChannel>,
    reply: Arc<RwLock<Option<WeComReplyRoute>>>,
    state: Mutex<WeComTurnState>,
}

fn cancel_debounce(state: &mut WeComTurnState) {
    if let Some(cancel) = state.debounce_cancel.take() {
        cancel.cancel();
    }
}

async fn push_draft(
    state: &mut WeComTurnState,
    channel: &WeComChannel,
    reply: &WeComReplyRoute,
    reason: &'static str,
) -> bool {
    let body_len = render_draft_body(state).len();
    let text = draft_update_display(&render_draft_body(state));
    let display_len = text.len();
    if text.is_empty() {
        tracing::debug!(
            reason,
            body_len,
            "WeCom draft push skipped: empty display"
        );
        return false;
    }

    let is_first = state.draft_msg_id.is_none();
    if is_first {
        match channel.start_draft(reply, &text).await {
            Ok(Some(new_id)) => state.draft_msg_id = Some(new_id),
            Ok(None) => {
                tracing::warn!(reason, body_len, display_len, "WeCom draft start returned None");
                return false;
            }
            Err(e) => {
                tracing::warn!(error = ?e, reason, body_len, display_len, "WeCom draft start failed");
                return false;
            }
        }
    }

    let Some(mid) = state.draft_msg_id.as_ref() else {
        return false;
    };
    match channel
        .update_draft(&reply.recipient, mid, &text, reason)
        .await
    {
        Ok(_) => {
            state.last_draft_push_at = Some(Instant::now());
            state.last_pushed_body_len = body_len;
            true
        }
        Err(e) => {
            tracing::warn!(
                error = ?e,
                reason,
                body_len,
                display_len,
                "WeCom draft push failed"
            );
            false
        }
    }
}

impl WeComSessionOutbound {
    pub fn new(channel: Arc<WeComChannel>) -> Arc<Self> {
        Arc::new(Self {
            channel,
            reply: Arc::new(RwLock::new(None)),
            state: Mutex::new(WeComTurnState::default()),
        })
    }

    pub async fn set_reply_route(self: &Arc<Self>, route: WeComReplyRoute) {
        *self.reply.write().await = Some(route);
    }

    pub async fn on_session_event(self: &Arc<Self>, event: &SessionEvent) {
        match &event.kind {
            SessionEventKind::AgentResponse { agent } => self.on_agent_event(agent).await,
            SessionEventKind::TurnFinish => self.on_turn_finish().await,
            SessionEventKind::Reset => {
                let mut state = self.state.lock().await;
                cancel_debounce(&mut state);
                if let Some(reply) = self.reply.read().await.clone() {
                    if let Some(mid) = state.draft_msg_id.take() {
                        let _ = self.channel.cancel_draft(&reply.recipient, &mid).await;
                    }
                    let ack = t!("channel-cmd-new-ok");
                    if let Err(e) = self.channel.deliver_plain_text(&reply, &ack).await {
                        tracing::warn!(error = %e, "WeCom failed to send reset ack");
                    }
                }
                *state = WeComTurnState::default();
            }
            SessionEventKind::TurnAccepted { .. } => {}
        }
    }

    async fn on_agent_event(self: &Arc<Self>, ev: &AgentResponseEvent) {
        match ev {
            AgentResponseEvent::CompletionResponse { chunk } => {
                if let Some(text) = chunk_text(chunk) {
                    self.handle_chunk(&text).await;
                }
            }
            AgentResponseEvent::ToolCall { event } => match &event.kind {
                ToolCallEventKind::Requested {
                    name,
                    arguments,
                } => {
                    let call_id = event.call_id.clone();
                    let display = friendly_tool_label(&name, &name);
                    let mut state = self.state.lock().await;
                    state.pending_auth.insert(
                        call_id.clone(),
                        PendingToolAuth {
                            tool_name: name.clone(),
                            display_name: display.clone(),
                            arguments: parse_tool_call_args(&arguments),
                        },
                    );
                    state.tool_displays.insert(call_id, display);
                }
                ToolCallEventKind::Started => {
                    let call_id = event.call_id.clone();
                    let display = {
                        let state = self.state.lock().await;
                        state
                            .tool_displays
                            .get(&call_id)
                            .cloned()
                            .unwrap_or_else(|| call_id.clone())
                    };
                    self.handle_tool_started(call_id, display).await;
                }
                ToolCallEventKind::Payload { text } => {
                    let mut state = self.state.lock().await;
                    let acc = state
                        .tool_outputs
                        .entry(event.call_id.clone())
                        .or_default();
                    acc.push_str(text);
                }
                ToolCallEventKind::Finished { status } => {
                    let call_id = event.call_id.clone();
                    let is_error = *status == ToolCallStatus::Error;
                    let output = {
                        let mut state = self.state.lock().await;
                        state.tool_outputs.remove(&call_id).unwrap_or_default()
                    };
                    let result = if is_error {
                        serde_json::json!({ "error": output })
                    } else {
                        serde_json::json!({ "output": output })
                    };
                    self.handle_tool_result(call_id, result, is_error).await;
                }
                ToolCallEventKind::Extra { .. } => {
                    self.handle_tool_auth(&event.call_id).await;
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

    async fn on_turn_finish(self: &Arc<Self>) {
        self.flush_draft_push("turn_finish_flush").await;

        let Some(reply) = self.reply.read().await.clone() else {
            let mut state = self.state.lock().await;
            cancel_debounce(&mut state);
            *state = WeComTurnState::default();
            return;
        };

        let (body_render, intermediate_render_len, draft_msg_id) = {
            let mut state = self.state.lock().await;
            cancel_debounce(&mut state);
            (
                render_draft_body(&state),
                state.intermediate_render_len,
                state.draft_msg_id.take(),
            )
        };

        if body_render.is_empty() {
            if let Some(mid) = draft_msg_id {
                let _ = self.channel.cancel_draft(&reply.recipient, &mid).await;
            }
        } else if let Some(mid) = draft_msg_id {
            let final_msg = build_finalize_payload(&body_render, intermediate_render_len);
            tracing::debug!(
                body_len = body_render.len(),
                final_len = final_msg.len(),
                stream_id = %mid,
                "WeCom draft finalize"
            );
            let _ = self
                .channel
                .finalize_draft(&reply.recipient, &mid, &final_msg)
                .await;
        } else {
            let _ = self.channel.deliver_plain_text(&reply, &body_render).await;
        }

        *self.state.lock().await = WeComTurnState::default();
    }

    async fn handle_chunk(self: &Arc<Self>, content: &str) {
        let stripped = {
            let mut state = self.state.lock().await;
            strip_think_tags(&mut state.think_tag_buf, content)
        };
        if stripped.is_empty() {
            return;
        }
        {
            let mut state = self.state.lock().await;
            state.body.push_str(&stripped);
        }
        self.request_draft_push(false).await;
    }

    async fn handle_tool_started(self: &Arc<Self>, call_id: String, display: String) {
        {
            let mut state = self.state.lock().await;
            if !state.body.is_empty() && !state.body.ends_with('\n') {
                state.body.push('\n');
            }
            state.body.push_str(&tool_placeholder(&call_id));
            state.body.push('\n');
            state
                .tools
                .insert(call_id, ToolSlot::Running { display });
        }
        self.request_draft_push(true).await;
        let mut state = self.state.lock().await;
        state.intermediate_render_len = render_draft_body(&state).len();
    }

    async fn handle_tool_result(
        self: &Arc<Self>,
        call_id: String,
        result: serde_json::Value,
        is_error: bool,
    ) {
        let preview = extract_output_preview(&result, 80);
        {
            let mut state = self.state.lock().await;
            if let Some(slot) = state.tools.get_mut(&call_id) {
                let display = match slot {
                    ToolSlot::Running { display } => display.clone(),
                    ToolSlot::Completed { display, .. } => display.clone(),
                };
                *slot = ToolSlot::Completed {
                    display,
                    success: !is_error,
                    preview,
                };
            }
        }
        self.request_draft_push(true).await;
        let mut state = self.state.lock().await;
        state.intermediate_render_len = render_draft_body(&state).len();
    }

    async fn request_draft_push(self: &Arc<Self>, immediate: bool) {
        let throttled = {
            let state = self.state.lock().await;
            let current_len = render_draft_body(&state).len();
            draft_push_throttled(
                state.last_draft_push_at,
                state.last_pushed_body_len,
                current_len,
                immediate,
            )
        };

        if immediate || !throttled {
            self.flush_draft_push(if immediate {
                "immediate"
            } else {
                "interval_or_delta"
            })
            .await;
            return;
        }

        let (cancel, already_debouncing) = {
            let mut state = self.state.lock().await;
            state.draft_push_pending = true;
            let already = state.debounce_cancel.is_some();
            cancel_debounce(&mut state);
            let cancel = CancellationToken::new();
            state.debounce_cancel = Some(cancel.clone());
            (cancel, already)
        };

        if already_debouncing {
            tracing::trace!("WeCom draft push throttled, debounce reset");
        }

        let outbound = Arc::clone(self);
        tokio::spawn(async move {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::debug!("WeCom draft debounce cancelled");
                }
                _ = tokio::time::sleep(Duration::from_millis(DRAFT_PUSH_MIN_INTERVAL_MS as u64)) => {
                    let Some(reply) = outbound.reply.read().await.clone() else {
                        return;
                    };
                    let mut state = outbound.state.lock().await;
                    if !state.draft_push_pending {
                        return;
                    }
                    state.draft_push_pending = false;
                    state.debounce_cancel = None;
                    let _ = push_draft(&mut state, &outbound.channel, &reply, "debounce").await;
                }
            }
        });
    }

    async fn flush_draft_push(self: &Arc<Self>, reason: &'static str) {
        let Some(reply) = self.reply.read().await.clone() else {
            tracing::debug!(reason, "WeCom draft push skipped: no reply route");
            return;
        };
        let mut state = self.state.lock().await;
        cancel_debounce(&mut state);
        state.draft_push_pending = false;
        let _ = push_draft(&mut state, &self.channel, &reply, reason).await;
    }

    async fn handle_tool_auth(self: &Arc<Self>, call_id: &str) {
        let pending = {
            let state = self.state.lock().await;
            if state.auth_prompted.contains(call_id) {
                return;
            }
            state.pending_auth.get(call_id).cloned()
        };
        let Some(pending) = pending else {
            tracing::warn!(call_id, "WeCom tool auth Extra without prior Requested event");
            return;
        };
        {
            let mut state = self.state.lock().await;
            state.auth_prompted.insert(call_id.to_string());
        }

        // moray：ApprovalNeeded 只发卡，不 finalize 流式草稿，避免企微侧 stream 被空 finish 打断。
        let Some(reply) = self.reply.read().await.clone() else {
            return;
        };
        if let Err(e) = self
            .channel
            .send_tool_approval_card(
                &reply.recipient,
                call_id,
                &pending.tool_name,
                &pending.display_name,
                &pending.arguments,
                reply.req_id.clone(),
            )
            .await
        {
            tracing::warn!(call_id, error = %e, "WeCom send_approval_prompt failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_finalize_payload, draft_push_throttled, split_body_at_render_boundary,
        DRAFT_PUSH_MIN_CHARS, DRAFT_PUSH_MIN_INTERVAL_MS,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn draft_push_throttle_skips_frequent_small_updates() {
        let last = Instant::now();
        assert!(draft_push_throttled(Some(last), 10, 20, false));
    }

    #[test]
    fn draft_push_throttle_allows_after_interval() {
        let last =
            Instant::now() - Duration::from_millis(DRAFT_PUSH_MIN_INTERVAL_MS as u64 + 1);
        assert!(!draft_push_throttled(Some(last), 10, 20, false));
    }

    #[test]
    fn draft_push_throttle_allows_large_delta() {
        let last = Instant::now();
        assert!(!draft_push_throttled(
            Some(last),
            0,
            DRAFT_PUSH_MIN_CHARS,
            false
        ));
    }

    #[test]
    fn draft_push_throttle_immediate_bypasses() {
        let last = Instant::now();
        assert!(!draft_push_throttled(Some(last), 10, 11, true));
    }

    #[test]
    fn draft_push_throttle_first_push_waits_for_char_threshold() {
        assert!(draft_push_throttled(None, 0, 5, false));
        assert!(!draft_push_throttled(None, 0, DRAFT_PUSH_MIN_CHARS, false));
    }

    #[test]
    fn split_body_no_tools_is_all_final() {
        let (think, body) = split_body_at_render_boundary("hello world", 0);
        assert!(think.is_empty());
        assert_eq!(body, "hello world");
    }

    #[test]
    fn split_body_after_tool_block_with_blank_line() {
        let input = "[✅ web_search]\n{\"preview\":true}\n\n深圳天气很好";
        let intermediate_len = "[✅ web_search]\n{\"preview\":true}\n\n".len();
        let (think, body) = split_body_at_render_boundary(input, intermediate_len);
        assert_eq!(think, "[✅ web_search]\n{\"preview\":true}");
        assert_eq!(body, "深圳天气很好");
    }

    #[test]
    fn split_body_after_tool_preview_without_blank_line() {
        let input = "[✅ web_search]\n{\"preview\":true}\n根据搜索结果，深圳今天气温 27-32°C";
        let intermediate_len = "[✅ web_search]\n{\"preview\":true}\n".len();
        let (think, body) = split_body_at_render_boundary(input, intermediate_len);
        assert_eq!(think, "[✅ web_search]\n{\"preview\":true}");
        assert_eq!(body, "根据搜索结果，深圳今天气温 27-32°C");
    }

    #[test]
    fn build_finalize_payload_no_tools_keeps_thinking_block() {
        let msg = build_finalize_payload("你好！有什么我可以帮你的吗？", 0);
        assert!(msg.contains("<think>"));
        assert!(msg.contains("</think>"));
        assert!(msg.contains("你好！有什么我可以帮你的吗？"));
        assert!(msg.matches("你好！有什么我可以帮你的吗？").count() >= 2);
    }

    #[test]
    fn build_finalize_payload_splits_think_and_body() {
        let input = "[✅ web_search]\n\n最终回答";
        let intermediate_len = "[✅ web_search]\n\n".len();
        let msg = build_finalize_payload(input, intermediate_len);
        assert!(msg.contains("<think>"));
        assert!(msg.contains("[✅ web_search]"));
        assert!(msg.contains("最终回答"));
        assert!(msg.contains("</think>"));
    }
}
