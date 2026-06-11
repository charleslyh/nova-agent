//! WeCom AI Bot channel — Rust port of `@wecom/aibot-node-sdk`.
//!
//! Connects to `wss://openws.work.weixin.qq.com` via WebSocket, authenticates
//! with `bot_id` + `bot_secret`, maintains heartbeat, and handles reconnection
//! with exponential back-off.
//!
//! Message flow:
//! 1. Connect → send auth frame (`aibot_subscribe`)
//! 2. Receive auth ACK → start heartbeat timer
//! 3. Receive message callbacks (`aibot_msg_callback`) → emit to channel
//! 4. Reply via `aibot_respond_msg` with stream / markdown / template_card

use crate::channels::render;
use moray_channels::{
    ApprovalDecision, AuthReply, ChannelError, ChannelRun, ImChannel, InboundMessage, UserMessage,
};
use crate::channels::wecom::reply::WeComReplyRoute;

use crate::channels::wecom::outbound::WeComSessionOutbound;
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

// ───────────────────── Constants ─────────────────────

const DEFAULT_WS_URL: &str = "wss://openws.work.weixin.qq.com";

/// WebSocket command constants — mirrors `WsCmd` from the Node SDK.
mod ws_cmd {
    pub const SUBSCRIBE: &str = "aibot_subscribe";
    pub const HEARTBEAT: &str = "ping";
    pub const RESPONSE: &str = "aibot_respond_msg";
    #[allow(dead_code)]
    pub const RESPONSE_WELCOME: &str = "aibot_respond_welcome_msg";
    pub const RESPONSE_UPDATE: &str = "aibot_respond_update_msg";
    pub const SEND_MSG: &str = "aibot_send_msg";
    pub const CALLBACK: &str = "aibot_msg_callback";
    pub const EVENT_CALLBACK: &str = "aibot_event_callback";
}

/// Maximum consecutive missed heartbeat ACKs before treating the connection as dead.
const MAX_MISSED_PONG: u32 = 2;
/// Upper cap for exponential back-off reconnect delay.
const RECONNECT_MAX_DELAY_MS: u64 = 30_000;
/// Reply ACK timeout.
const REPLY_ACK_TIMEOUT: Duration = Duration::from_secs(5);

// ───────────────────── Types ─────────────────────

/// WeCom WebSocket frame (sent & received).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WsFrame {
    #[serde(skip_serializing_if = "Option::is_none")]
    cmd: Option<String>,
    #[serde(default)]
    headers: WsHeaders,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    errcode: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    errmsg: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct WsHeaders {
    #[serde(default)]
    req_id: String,
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

/// Message type enum — mirrors the Node SDK `MessageType`.
#[derive(Debug, Clone, PartialEq)]
enum MessageType {
    Text,
    Image,
    Mixed,
    Voice,
    File,
    Event,
    Unknown(String),
}

impl MessageType {
    fn from_str(s: &str) -> Self {
        match s {
            "text" => Self::Text,
            "image" => Self::Image,
            "mixed" => Self::Mixed,
            "voice" => Self::Voice,
            "file" => Self::File,
            "event" => Self::Event,
            other => Self::Unknown(other.to_string()),
        }
    }
}

// ───────────────────── Utilities ─────────────────────

/// Generate a unique request ID: `{prefix}_{timestamp_ms}_{random_hex}`.
fn generate_req_id(prefix: &str) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let random = &Uuid::new_v4().to_string()[..8];
    format!("{prefix}_{ts}_{random}")
}

/// Convert `[IMAGE:url]` markers and plain image URLs to standard markdown image syntax `![](url)`.
fn convert_image_markers_to_markdown(content: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;

    // Step 1: Convert [IMAGE:url] markers
    static IMAGE_MARKER_REGEX: OnceLock<Regex> = OnceLock::new();
    let marker_regex =
        IMAGE_MARKER_REGEX.get_or_init(|| Regex::new(r"\[IMAGE:([^\]]+)\]").unwrap());

    let content = marker_regex.replace_all(content, "![](<$1>)");

    // Step 2: Convert plain image URLs that are not already in markdown format
    // Rust regex doesn't support lookbehind, so we use replacer function to check context
    static PLAIN_IMAGE_URL_REGEX: OnceLock<Regex> = OnceLock::new();
    let url_regex = PLAIN_IMAGE_URL_REGEX.get_or_init(|| {
        // Match image URLs (ending with common image extensions)
        Regex::new(r"https?://[^\s\)\]]+\.(?:png|jpg|jpeg|gif|webp|bmp|svg)(?:\?[^\s\)\]]*)?")
            .unwrap()
    });

    // Check if URL is already wrapped in markdown image syntax
    let is_already_markdown = |content: &str, start: usize| -> bool {
        if start >= 2 {
            let prefix = &content[start.saturating_sub(2)..start];
            // Check for standard syntax ![](url) or angle bracket syntax ![](<url>)
            if prefix.ends_with("![") || prefix.ends_with("](") || prefix.ends_with("(<") {
                return true;
            }
        }
        false
    };

    let content_str = content.as_ref();
    let mut result = String::with_capacity(content_str.len() + 100);
    let mut last_end = 0;

    for mat in url_regex.find_iter(content_str) {
        let start = mat.start();
        let end = mat.end();

        // Append text before this match
        result.push_str(&content_str[last_end..start]);

        if is_already_markdown(content_str, start) {
            // Already in markdown format, keep as-is
            result.push_str(mat.as_str());
        } else {
            // Wrap in markdown image syntax
            result.push_str("![](");
            result.push_str(mat.as_str());
            result.push(')');
        }
        last_end = end;
    }

    // Append remaining text
    result.push_str(&content_str[last_end..]);

    result
}

// ───────────────────── Pending ACK tracking ─────────────────────

/// A one-shot reply-ACK tracker for serialised reply sending.
struct PendingAck {
    tx: tokio::sync::oneshot::Sender<Result<WsFrame, String>>,
}

// ───────────────────── WeComChannel ─────────────────────

/// Card event context with creation timestamp for TTL management.
/// Used to store req_id from template_card_event for later card updates via WebSocket.
#[derive(Clone)]
struct CardEventEntry {
    /// The req_id from the event frame, needed for WebSocket card update
    req_id: Option<String>,
    /// Tool name for display in updated card
    tool_name: String,
    /// The i18n display name for the tool (already resolved via friendly_tool_label).
    tool_display_name: String,
    /// Arguments preview for display in updated card
    args_preview: String,
    created_at: std::time::Instant,
}

/// TTL for card event entries (WeCom requires response within 5 seconds, but we allow some buffer)
/// However, for approval workflows, the user may take longer to decide, so we use a generous TTL.
const CARD_EVENT_ENTRY_TTL: Duration = Duration::from_secs(60 * 60); // 1 hour

/// Tracks active stream draft for proper ordering with approval cards.
#[derive(Clone, Default)]
struct ActiveDraft {
    /// 是否真的发过内容到 WeCom 端。`send_draft` 只占位，不发内容；
    /// 第一次成功 `update_draft` 才置 true。`cancel_draft` 用此判断
    /// 是否需要发"已取消"消息（false 时静默丢弃，避免凭空冒消息）。
    has_content: bool,
}

/// The main WeCom channel implementation.
pub struct WeComChannel {
    bot_id: String,
    bot_secret: String,
    heartbeat_interval: Duration,
    max_reconnect_attempts: u32,
    /// Directory for saving downloaded media files.
    workspace_dir: PathBuf,
    /// Shared write-half of the WebSocket — set after connection.
    ws_writer: Arc<RwLock<Option<WsWriter>>>,
    /// Pending ACK map: req_id → oneshot sender.
    pending_acks: Arc<RwLock<HashMap<String, PendingAck>>>,
    /// Card event entries: task_id → CardEventEntry for updating cards after approval
    card_event_entries: Arc<RwLock<HashMap<String, CardEventEntry>>>,
    /// Active drafts: req_id → ActiveDraft (supports multiple concurrent users)
    active_drafts: Arc<RwLock<HashMap<String, ActiveDraft>>>,
    /// Last response_url from template_card_event, keyed by reply_target.
    /// Used to send final answer after approval workflow completes (appears below approval cards).
    last_response_url: Arc<RwLock<HashMap<String, String>>>,
    session_outbound: Arc<RwLock<Option<Arc<WeComSessionOutbound>>>>,
}

type WsWriter = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

impl WeComChannel {
    pub fn new(
        bot_id: String,
        bot_secret: String,
        workspace_dir: PathBuf,
    ) -> Self {
        Self {
            bot_id,
            bot_secret,
            heartbeat_interval: Duration::from_millis(30_000),
            max_reconnect_attempts: 10,
            workspace_dir,
            ws_writer: Arc::new(RwLock::new(None)),
            pending_acks: Arc::new(RwLock::new(HashMap::new())),
            card_event_entries: Arc::new(RwLock::new(HashMap::new())),
            active_drafts: Arc::new(RwLock::new(HashMap::new())),
            last_response_url: Arc::new(RwLock::new(HashMap::new())),
            session_outbound: Arc::new(RwLock::new(None)),
        }
    }

    async fn init_outbound(self: &Arc<Self>) {
        let mut slot = self.session_outbound.write().await;
        if slot.is_none() {
            *slot = Some(WeComSessionOutbound::new(Arc::clone(self)));
        }
    }

    pub(crate) async fn deliver_plain_text(
        &self,
        reply: &WeComReplyRoute,
        content: &str,
    ) -> Result<(), ChannelError> {
        let req_id = reply.req_id.as_deref().unwrap_or("");
        if req_id.is_empty() {
            let chatid = reply
                .recipient
                .strip_prefix("user:")
                .or_else(|| reply.recipient.strip_prefix("group:"))
                .unwrap_or(&reply.recipient);
            return self.send_proactive_message(chatid, content).await;
        }
        let stream_id = generate_req_id("stream");
        self.reply_stream(req_id, &stream_id, content, true).await
    }

    pub(crate) async fn start_draft(
        &self,
        reply: &WeComReplyRoute,
        _text: &str,
    ) -> Result<Option<String>, ChannelError> {
        let req_id = reply.req_id.as_deref().unwrap_or("");
        if req_id.is_empty() {
            return Ok(None);
        }
        let stream_id = generate_req_id("stream");
        // 与 moray send_draft 一致：仅占位 stream.id，首帧 WS 留到第一次 update_draft。
        {
            let mut drafts = self.active_drafts.write().await;
            drafts.insert(req_id.to_string(), ActiveDraft::default());
        }
        Ok(Some(format!("{req_id}|{stream_id}")))
    }

    pub fn with_heartbeat_interval(mut self, ms: u64) -> Self {
        self.heartbeat_interval = Duration::from_millis(ms);
        self
    }

    pub fn with_max_reconnect_attempts(mut self, n: u32) -> Self {
        self.max_reconnect_attempts = n;
        self
    }

    // ─── WebSocket send helpers ───

    /// Send a raw frame through the WebSocket.
    async fn ws_send(&self, frame: &WsFrame) -> Result<(), ChannelError> {
        let mut writer = self.ws_writer.write().await;
        let ws = writer
            .as_mut()
            .ok_or_else(|| ChannelError::op("WeCom: WebSocket not connected"))?;
        let text = serde_json::to_string(frame)?;
        ws.send(Message::Text(text.into())).await?;
        Ok(())
    }

    /// Send the authentication frame.
    async fn send_auth(&self) -> Result<(), ChannelError> {
        let frame = WsFrame {
            cmd: Some(ws_cmd::SUBSCRIBE.to_string()),
            headers: WsHeaders {
                req_id: generate_req_id(ws_cmd::SUBSCRIBE),
                extra: HashMap::new(),
            },
            body: Some(json!({
                "bot_id": self.bot_id,
                "secret": self.bot_secret,
            })),
            errcode: None,
            errmsg: None,
        };
        self.ws_send(&frame).await?;
        tracing::info!("WeCom: auth frame sent");
        Ok(())
    }

    /// Send a heartbeat ping.
    async fn send_heartbeat(&self) -> Result<(), ChannelError> {
        let frame = WsFrame {
            cmd: Some(ws_cmd::HEARTBEAT.to_string()),
            headers: WsHeaders {
                req_id: generate_req_id(ws_cmd::HEARTBEAT),
                extra: HashMap::new(),
            },
            body: None,
            errcode: None,
            errmsg: None,
        };
        self.ws_send(&frame).await
    }

    /// Send a reply frame with the given req_id and body.
    /// Returns when ACK is received or timeout.
    async fn send_reply(&self, req_id: &str, body: Value, cmd: &str) -> Result<(), ChannelError> {
        let frame = WsFrame {
            cmd: Some(cmd.to_string()),
            headers: WsHeaders {
                req_id: req_id.to_string(),
                extra: HashMap::new(),
            },
            body: Some(body),
            errcode: None,
            errmsg: None,
        };

        // Register pending ACK.
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = self.pending_acks.write().await;
            pending.insert(req_id.to_string(), PendingAck { tx });
        }

        // Send the frame.
        if let Err(e) = self.ws_send(&frame).await {
            // Remove pending on send failure.
            self.pending_acks.write().await.remove(req_id);
            return Err(e);
        }

        tracing::debug!(req_id, "WeCom: reply sent, waiting for ACK");

        // Wait for ACK with timeout.
        match tokio::time::timeout(REPLY_ACK_TIMEOUT, rx).await {
            Ok(Ok(Ok(_ack_frame))) => {
                tracing::debug!(req_id, "WeCom: reply ACK received");
                Ok(())
            }
            Ok(Ok(Err(err_msg))) => {
                tracing::warn!(req_id, err = %err_msg, "WeCom: reply ACK error");
                Err(ChannelError::op(format!("WeCom reply ACK error: {err_msg}")))
            }
            Ok(Err(_)) => {
                // oneshot dropped — connection closed
                self.pending_acks.write().await.remove(req_id);
                Err(ChannelError::op(
                    "WeCom: connection closed while waiting for ACK"
                ))
            }
            Err(_) => {
                // Timeout
                self.pending_acks.write().await.remove(req_id);
                tracing::warn!(req_id, "WeCom: reply ACK timeout");
                // Don't error out — the message was sent, just no ACK.
                Ok(())
            }
        }
    }

    /// Send a stream reply (convenience wrapper).
    async fn reply_stream(
        &self,
        req_id: &str,
        stream_id: &str,
        content: &str,
        finish: bool,
    ) -> Result<(), ChannelError> {
        // Convert [IMAGE:url] markers to standard markdown image syntax
        let content = convert_image_markers_to_markdown(content);
        if finish {
            tracing::debug!(
                req_id,
                stream_id,
                content_preview = %content,
                finished = %finish,
                "WeCom: reply_stream"
            );
        }
        let body = json!({
            "msgtype": "stream",
            "stream": {
                "id": stream_id,
                "finish": finish,
                "content": content,
            }
        });
        self.send_reply(req_id, body, ws_cmd::RESPONSE).await
    }

    /// Reply to `enter_chat` within the 5s window (`aibot_respond_welcome_msg`).
    async fn reply_welcome(&self, req_id: &str, content: &str) -> Result<(), ChannelError> {
        let body = json!({
            "msgtype": "text",
            "text": { "content": content }
        });
        tracing::info!(%req_id, "WeCom: sending enter_chat welcome");
        self.send_reply(req_id, body, ws_cmd::RESPONSE_WELCOME).await
    }

    /// Send a proactive message to a user or group (not a reply).
    ///
    /// This uses `aibot_send_msg` command which allows sending messages
    /// without a prior incoming message.
    ///
    /// - `chatid`: For single chat, use the user's `userid`. For group chat, use the `chatid`.
    /// - `content`: Markdown content to send.
    pub async fn send_proactive_message(&self, chatid: &str, content: &str) -> Result<(), ChannelError> {
        // Convert [IMAGE:url] markers to standard markdown image syntax
        let content = convert_image_markers_to_markdown(content);
        let req_id = generate_req_id(ws_cmd::SEND_MSG);
        let body = json!({
            "chatid": chatid,
            "msgtype": "markdown",
            "markdown": {
                "content": content,
            }
        });
        self.send_reply(&req_id, body, ws_cmd::SEND_MSG).await
    }

    /// POST markdown to `response_url` from template_card_event (appears below approval card).
    async fn send_via_response_url(&self, url: &str, content: &str) -> Result<(), ChannelError> {
        let content = convert_image_markers_to_markdown(content);
        let body = json!({
            "msgtype": "markdown",
            "markdown": {
                "content": content,
            }
        });
        let client = reqwest::Client::new();
        let resp = client
            .post(url)
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ChannelError::op(format!(
                "response_url returned HTTP {status}: {text}"
            )));
        }
        tracing::info!("WeCom: sent final answer via response_url");
        Ok(())
    }

    /// Send an approval template card with "Allow" and "Deny" buttons.
    ///
    /// This uses the `button_interaction` card type from WeCom AI Bot SDK.
    /// When a user clicks a button, a `template_card_event` callback is sent.
    ///
    /// - `chatid`: Target user or group.
    /// - `request_id`: Unique approval request ID (used as `task_id`).
    /// - `tool_name`: Name of the tool requiring approval.
    /// - `args_preview`: Preview of the tool arguments.
    /// - `thread_ts`: Optional original message req_id for passive reply.
    /// Pre-store card metadata and send an approval template card.
    pub async fn send_tool_approval_card(
        &self,
        recipient: &str,
        request_id: &str,
        tool_name: &str,
        tool_display_name: &str,
        arguments: &serde_json::Value,
        thread_anchor: Option<String>,
    ) -> Result<(), ChannelError> {
        let chatid = recipient
            .strip_prefix("user:")
            .or_else(|| recipient.strip_prefix("group:"))
            .unwrap_or(recipient);

        let raw_args = arguments.to_string();
        let args_preview = if raw_args.len() > 200 {
            let end = render::floor_utf8_char_boundary(&raw_args, 200);
            format!("{}...", &raw_args[..end])
        } else {
            raw_args
        };

        {
            let mut entries = self.card_event_entries.write().await;
            entries.retain(|_, entry| entry.created_at.elapsed() < CARD_EVENT_ENTRY_TTL);
            entries.insert(
                request_id.to_string(),
                CardEventEntry {
                    req_id: None,
                    tool_name: tool_name.to_string(),
                    tool_display_name: tool_display_name.to_string(),
                    args_preview: args_preview.clone(),
                    created_at: std::time::Instant::now(),
                },
            );
        }

        self.send_approval_template_card(
            chatid,
            request_id,
            tool_name,
            tool_display_name,
            &args_preview,
            thread_anchor.as_deref(),
        )
        .await
    }

    pub async fn send_approval_template_card(
        &self,
        chatid: &str,
        request_id: &str,
        tool_name: &str,
        tool_display_name: &str,
        args_preview: &str,
        thread_ts: Option<&str>,
    ) -> Result<(), ChannelError> {
        // Convert internal prefixes (seatbelt:/path_permission:) to i18n-friendly labels
        let display_name = render::friendly_tool_label(tool_name, tool_display_name);
        let card = json!({
            "card_type": "button_interaction",
            "main_title": {
                "title": t!("wecom-approval-title"),
                "desc": t!("wecom-approval-desc", tool = &display_name)
            },
            "sub_title_text": t!("wecom-sub-title-text", tool = &display_name, args = args_preview),
            "task_id": request_id,
            "button_list": [
                {
                    "text": t!("wecom-btn-allow"),
                    "style": 1,
                    "key": format!("approve_allow_{}", request_id)
                },
                {
                    "text": t!("wecom-btn-deny"),
                    "style": 2,
                    "key": format!("approve_deny_{}", request_id)
                }
            ]
        });

        let body = json!({
            "chatid": chatid,
            "msgtype": "template_card",
            "template_card": card
        });

        // Decide whether to use passive reply or proactive send
        if let Some(req_id) = thread_ts {
            if !req_id.is_empty() {
                // Passive reply using the original req_id
                self.send_reply(req_id, body, ws_cmd::RESPONSE).await
            } else {
                // Proactive send
                let req_id = generate_req_id(ws_cmd::SEND_MSG);
                self.send_reply(&req_id, body, ws_cmd::SEND_MSG).await
            }
        } else {
            // Proactive send
            let req_id = generate_req_id(ws_cmd::SEND_MSG);
            self.send_reply(&req_id, body, ws_cmd::SEND_MSG).await
        }
    }

    /// Update an approval card button after user action.
    ///
    /// Uses the stored `req_id` from the card event callback to update the card via WebSocket.
    /// According to WeCom AI Bot SDK, card updates must be sent through WebSocket using
    /// the `aibot_respond_update_msg` command with the original event's `req_id`.
    ///
    /// - `task_id`: The approval request ID (used to look up stored req_id).
    /// - `approved`: Whether the request was approved or denied.
    pub async fn update_approval_card(&self, task_id: &str, approved: bool) -> Result<(), ChannelError> {
        // Get and remove the card event entry (req_id can only be used once per event)
        let entry = {
            let mut entries = self.card_event_entries.write().await;
            entries.remove(task_id)
        };

        let (req_id, tool_name, tool_display_name, args_preview) = match entry {
            Some(e) => {
                // Check if the entry has expired
                if e.created_at.elapsed() >= CARD_EVENT_ENTRY_TTL {
                    tracing::warn!(
                        task_id,
                        "WeCom: card event entry has expired, cannot update card"
                    );
                    return Ok(());
                }
                match e.req_id {
                    Some(rid) => (rid, e.tool_name, e.tool_display_name, e.args_preview),
                    None => {
                        tracing::warn!(
                            task_id,
                            "WeCom: req_id not set in card entry, cannot update card"
                        );
                        return Ok(());
                    }
                }
            }
            None => {
                tracing::warn!(
                    task_id,
                    "WeCom: no card entry found for task_id, cannot update card"
                );
                return Ok(());
            }
        };

        let display_label = render::friendly_tool_label(&tool_name, &tool_display_name);

        let (status_emoji, status_text) = if approved {
            ("✅", t!("wecom-status-allowed"))
        } else {
            ("❌", t!("wecom-status-denied"))
        };

        // Build the update body according to AI Bot SDK format
        // response_type must be "update_template_card" and card_type must match original
        // Include tool_name and args_preview so user knows what was approved/denied
        let body = json!({
            "response_type": "update_template_card",
            "template_card": {
                "card_type": "button_interaction",
                "task_id": task_id,
                "main_title": {
                    "title": t!("wecom-approval-status-title", emoji = status_emoji),
                    "desc": status_text
                },
                "sub_title_text": t!("wecom-sub-title-text", tool = &display_label, args = &args_preview),
                "button_list": [
                    {
                        "text": if approved { t!("wecom-btn-allowed") } else { t!("wecom-btn-denied") },
                        "style": if approved { 1 } else { 2 },
                        "key": format!("approved_{}", task_id)
                    }
                ]
            }
        });

        tracing::info!(task_id, approved, status_text, %req_id, %tool_name, "WeCom: updating approval card via WebSocket");

        // Send via WebSocket using aibot_respond_update_msg command
        self.send_reply(&req_id, body, ws_cmd::RESPONSE_UPDATE)
            .await
    }

    // ─── Frame handling ───

    /// Process a single incoming frame, dispatch to appropriate handler.
    async fn handle_frame(
        self: &Arc<Self>,
        frame: WsFrame,
        authenticated: &mut bool,
        missed_pong: &mut u32,
        inbound_tx: &mpsc::Sender<InboundMessage>,
    ) {
        // 1. Message push callback
        if frame.cmd.as_deref() == Some(ws_cmd::CALLBACK) {
            self.handle_message_callback(&frame, inbound_tx);
            return;
        }

        // 2. Event push callback
        if frame.cmd.as_deref() == Some(ws_cmd::EVENT_CALLBACK) {
            self.handle_event_callback(&frame, inbound_tx).await;
            return;
        }

        // 3. No cmd → auth/heartbeat ACK or reply ACK
        let req_id = &frame.headers.req_id;

        // Check pending ACKs first (reply receipts)
        {
            let pending_acks = self.pending_acks.clone();
            let req_id_owned = req_id.clone();
            let frame_clone = frame.clone();
            tokio::spawn(async move {
                let mut pending = pending_acks.write().await;
                if let Some(ack) = pending.remove(&req_id_owned) {
                    if frame_clone.errcode.unwrap_or(0) != 0 {
                        let msg = format!(
                            "errcode={}, errmsg={}",
                            frame_clone.errcode.unwrap_or(-1),
                            frame_clone.errmsg.as_deref().unwrap_or("unknown")
                        );
                        let _ = ack.tx.send(Err(msg));
                    } else {
                        let _ = ack.tx.send(Ok(frame_clone));
                    }
                }
            });
        }

        // Auth response
        if req_id.starts_with(ws_cmd::SUBSCRIBE) {
            if frame.errcode.unwrap_or(-1) != 0 {
                tracing::error!(
                    errcode = frame.errcode,
                    errmsg = ?frame.errmsg,
                    "WeCom: authentication failed"
                );
            } else {
                tracing::info!("WeCom: authentication successful");
                *authenticated = true;
            }
            return;
        }

        // Heartbeat ACK
        if req_id.starts_with(ws_cmd::HEARTBEAT) {
            if frame.errcode.unwrap_or(0) != 0 {
                tracing::warn!(
                    errcode = frame.errcode,
                    errmsg = ?frame.errmsg,
                    "WeCom: heartbeat ACK error"
                );
            } else {
                *missed_pong = 0;
                tracing::debug!("WeCom: heartbeat ACK received");
            }
            return;
        }

        // Unknown frame
        tracing::debug!(
            req_id,
            cmd = ?frame.cmd,
            "WeCom: received unknown frame"
        );
    }

    /// Handle a message callback (`aibot_msg_callback`).
    fn handle_message_callback(
        self: &Arc<Self>,
        frame: &WsFrame,
        inbound_tx: &mpsc::Sender<InboundMessage>,
    ) {
        let body = match &frame.body {
            Some(b) => b,
            None => {
                tracing::warn!("WeCom: message callback without body");
                return;
            }
        };

        let msgtype_str = body.get("msgtype").and_then(|v| v.as_str()).unwrap_or("");
        let msgtype = MessageType::from_str(msgtype_str);

        // Debug: log the message type and raw body structure
        tracing::debug!(
            msgtype = msgtype_str,
            has_mixed = body.get("mixed").is_some(),
            has_file = body.get("file").is_some(),
            has_image = body.get("image").is_some(),
            has_attachments = body.get("attachments").is_some(),
            body_keys = ?body.as_object().map(|o| o.keys().collect::<Vec<_>>()),
            "WeCom: parsing incoming message"
        );

        // Collect media download tasks for unified processing
        let mut media_tasks: Vec<MediaDownloadTask> = Vec::new();

        // Helper closure: extract image task from a JSON value (image object)
        fn extract_image_task(image_obj: &Value, tasks: &mut Vec<MediaDownloadTask>) {
            let url = image_obj.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if url.is_empty() {
                return;
            }
            let aes_key = image_obj
                .get("aeskey")
                .and_then(|k| k.as_str())
                .filter(|k| !k.is_empty())
                .map(|k| k.to_string());
            let placeholder = format!("[WECOM_MEDIA:{}]", url);
            tasks.push(MediaDownloadTask {
                url: url.to_string(),
                aes_key,
                is_image: true,
                placeholder,
            });
        }

        // Helper closure: extract file task from a JSON value (file object)
        fn extract_file_task(file_obj: &Value, tasks: &mut Vec<MediaDownloadTask>) {
            let url = file_obj.get("url").and_then(|u| u.as_str()).unwrap_or("");
            if url.is_empty() {
                return;
            }
            let aes_key = file_obj
                .get("aeskey")
                .and_then(|k| k.as_str())
                .filter(|k| !k.is_empty())
                .map(|k| k.to_string());
            let placeholder = format!("[WECOM_MEDIA:{}]", url);
            tasks.push(MediaDownloadTask {
                url: url.to_string(),
                aes_key,
                is_image: false,
                placeholder,
            });
        }

        // Extract text content from various message types.
        let content = match msgtype {
            MessageType::Text => {
                let mut text = body
                    .get("text")
                    .and_then(|t| t.get("content"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                // Some WeCom clients send file/image in a text message with attachments
                if let Some(attachments) = body.get("attachments").and_then(|a| a.as_array()) {
                    for att in attachments {
                        let att_type = att.get("msgtype").and_then(|v| v.as_str()).unwrap_or("");
                        if att_type == "file" {
                            if let Some(file_obj) = att.get("file") {
                                extract_file_task(file_obj, &mut media_tasks);
                                if let Some(task) = media_tasks.last() {
                                    if !text.is_empty() {
                                        text.push('\n');
                                    }
                                    text.push_str(&task.placeholder);
                                }
                            }
                        } else if att_type == "image" {
                            if let Some(image_obj) = att.get("image") {
                                extract_image_task(image_obj, &mut media_tasks);
                                if let Some(task) = media_tasks.last() {
                                    if !text.is_empty() {
                                        text.push('\n');
                                    }
                                    text.push_str(&task.placeholder);
                                }
                            }
                        }
                    }
                }
                text
            }
            MessageType::Voice => body
                .get("voice")
                .and_then(|v| v.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string(),
            MessageType::Mixed => {
                // Concatenate text parts from mixed messages.
                let mut text_parts = Vec::new();
                if let Some(items) = body
                    .get("mixed")
                    .and_then(|m| m.get("msg_item"))
                    .and_then(|a| a.as_array())
                {
                    for item in items {
                        let item_type = item.get("msgtype").and_then(|v| v.as_str()).unwrap_or("");
                        match item_type {
                            "text" => {
                                if let Some(t) = item
                                    .get("text")
                                    .and_then(|t| t.get("content"))
                                    .and_then(|c| c.as_str())
                                {
                                    text_parts.push(t.to_string());
                                }
                            }
                            "image" => {
                                if let Some(image_obj) = item.get("image") {
                                    extract_image_task(image_obj, &mut media_tasks);
                                    if let Some(task) = media_tasks.last() {
                                        text_parts.push(task.placeholder.clone());
                                    }
                                }
                            }
                            "file" => {
                                if let Some(file_obj) = item.get("file") {
                                    extract_file_task(file_obj, &mut media_tasks);
                                    if let Some(task) = media_tasks.last() {
                                        text_parts.push(task.placeholder.clone());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                text_parts.join("\n")
            }
            MessageType::Image => {
                if let Some(image_obj) = body.get("image") {
                    extract_image_task(image_obj, &mut media_tasks);
                    media_tasks
                        .last()
                        .map(|t| t.placeholder.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            }
            MessageType::File => {
                if let Some(file_obj) = body.get("file") {
                    extract_file_task(file_obj, &mut media_tasks);
                    media_tasks
                        .last()
                        .map(|t| t.placeholder.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            }
            _ => {
                tracing::debug!(msgtype = msgtype_str, "WeCom: unhandled message type");
                return;
            }
        };

        if content.is_empty() {
            tracing::debug!(msgtype = msgtype_str, "WeCom: empty content, skipping");
            return;
        }

        // Extract sender info.
        let sender = body
            .get("from")
            .and_then(|f| f.get("userid"))
            .and_then(|u| u.as_str())
            .unwrap_or("unknown")
            .to_string();

        let chatid = body
            .get("chatid")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        let chattype = body
            .get("chattype")
            .and_then(|c| c.as_str())
            .unwrap_or("single");

        // reply_target: for group chats use chatid, for single use sender userid.
        let reply_target = if chattype == "group" {
            format!("group:{}", chatid.as_deref().unwrap_or(&sender))
        } else {
            format!("user:{sender}")
        };

        let req_id = frame.headers.req_id.clone();

        let inbound_tx = inbound_tx.clone();
        let channel = Arc::clone(self);
        let workspace_dir = self.workspace_dir.clone();
        let route = WeComReplyRoute::new(reply_target.clone(), Some(req_id.clone()));

        // Spawn async task to handle media download and message dispatch
        tokio::spawn(async move {
            channel.store_reply_route(route.clone()).await;

            let mut final_content = content;

            // Process media download tasks: download (+ decrypt if needed) → save to local
            if !media_tasks.is_empty() {
                let save_dir = workspace_dir
                    .join("channels")
                    .join(format!("wecom_{}", sender));
                if let Err(e) = tokio::fs::create_dir_all(&save_dir).await {
                    tracing::warn!(err = %e, dir = %save_dir.display(), "WeCom: failed to create media dir");
                }

                for task in &media_tasks {
                    match download_and_save_media(task, &save_dir).await {
                        Ok(save_path) => {
                            let label = if task.is_image { t!("wecom-label-image") } else { t!("wecom-label-file") };
                            tracing::info!(
                                url = %task.url,
                                path = %save_path.display(),
                                "WeCom: media saved locally"
                            );
                            final_content = final_content.replace(
                                &task.placeholder,
                                &t!("wecom-upload-path", label = &label, path = &save_path.display().to_string()),
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                url = %task.url,
                                err = %e,
                                "WeCom: media download/save failed, using original URL"
                            );
                            let label = if task.is_image { t!("wecom-label-image") } else { t!("wecom-label-file") };
                            final_content = final_content.replace(
                                &task.placeholder,
                                &t!("wecom-upload-link", label = &label, url = &task.url),
                            );
                        }
                    }
                }
            }

            let _ = inbound_tx
                .send(InboundMessage::User(UserMessage {
                    content: final_content,
                }))
                .await;
        });
    }

    async fn store_reply_route(self: &Arc<Self>, route: WeComReplyRoute) {
        if let Some(out) = self.session_outbound.read().await.as_ref() {
            out.set_reply_route(route).await;
        }
    }

    /// Handle an event callback (`aibot_event_callback`).
    async fn handle_event_callback(
        self: &Arc<Self>,
        frame: &WsFrame,
        inbound_tx: &mpsc::Sender<InboundMessage>,
    ) {
        let body = match &frame.body {
            Some(b) => b,
            None => return,
        };

        let event = match body.get("event") {
            Some(e) => e,
            None => return,
        };

        let event_type = event
            .get("eventtype")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown");

        // Get req_id from frame headers for card updates
        let req_id = frame.headers.req_id.clone();

        tracing::info!(event_type, %req_id, "WeCom: received event callback");

        match event_type {
            "enter_chat" => {
                let userid = body
                    .get("from")
                    .and_then(|f| f.get("userid"))
                    .and_then(|u| u.as_str())
                    .unwrap_or("");
                tracing::info!(userid, "WeCom: user entered chat");
                let welcome = t!("wecom-welcome-default");
                if let Err(e) = self.reply_welcome(&req_id, &welcome).await {
                    tracing::warn!(err = %e, %req_id, "WeCom: enter_chat welcome failed");
                }
            }
            "disconnected_event" => {
                tracing::info!(
                    %req_id,
                    "WeCom: disconnected_event (another client took over this bot connection)"
                );
            }
            "template_card_event" => {
                // Debug: print the full event to see its structure
                tracing::debug!(?event, ?body, %req_id, "WeCom: template_card_event raw data");
                // Parse template card button click event and convert to approval command
                self.handle_template_card_event(event, body, &req_id, inbound_tx);
            }
            "feedback_event" => {
                let feedback = event.get("feedback_event");
                tracing::info!(
                    ?feedback,
                    userid = body
                        .get("from")
                        .and_then(|f| f.get("userid"))
                        .and_then(|u| u.as_str()),
                    "WeCom: user feedback on bot reply"
                );
            }
            _ => {
                tracing::debug!(event_type, "WeCom: unhandled event type");
            }
        }
    }

    /// Handle a template card button click event.
    ///
    /// Actual event structure from WeCom AI Bot SDK:
    /// ```json
    /// {
    ///   "event": {
    ///     "eventtype": "template_card_event",
    ///     "template_card_event": {
    ///       "card_type": "button_interaction",
    ///       "event_key": "approve_allow_apr-xxx",
    ///       "task_id": "apr-xxx"
    ///     }
    ///   },
    ///   "from": { "userid": "xxx" },
    ///   "chattype": "single"
    /// }
    /// ```
    fn handle_template_card_event(
        self: &Arc<Self>,
        event: &Value,
        body: &Value,
        req_id: &str,
        inbound_tx: &mpsc::Sender<InboundMessage>,
    ) {
        // WeCom actual structure:
        // event.template_card_event.task_id
        // event.template_card_event.event_key (the button key)
        let card_event = match event.get("template_card_event") {
            Some(ce) => ce,
            None => {
                tracing::warn!("WeCom: template card event without template_card_event object");
                return;
            }
        };

        let task_id = card_event
            .get("task_id")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();

        if task_id.is_empty() {
            tracing::warn!("WeCom: template card event without task_id");
            return;
        }

        // event_key is the button key (e.g., "approve_allow_apr-xxx")
        let button_key = card_event
            .get("event_key")
            .and_then(|k| k.as_str())
            .unwrap_or("")
            .to_string();

        if button_key.is_empty() {
            tracing::warn!(task_id, "WeCom: template card event without event_key");
            return;
        }

        // Extract user info
        let userid = body
            .get("from")
            .and_then(|f| f.get("userid"))
            .and_then(|u| u.as_str())
            .unwrap_or("unknown")
            .to_string();

        let chatid = body
            .get("chatid")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        // Update the existing card entry with req_id for later card update via WebSocket
        // This is required by WeCom AI Bot SDK - card updates must use the event's req_id
        // The entry was pre-created in send_approval_prompt with tool info
        {
            let card_entries = self.card_event_entries.clone();
            let task_id_clone = task_id.clone();
            let req_id_owned = req_id.to_string();
            tokio::spawn(async move {
                let mut entries = card_entries.write().await;
                // Clean up expired entries while we're here
                entries.retain(|_, entry| entry.created_at.elapsed() < CARD_EVENT_ENTRY_TTL);
                // Update existing entry with req_id (preserve tool_name and args_preview)
                if let Some(entry) = entries.get_mut(&task_id_clone) {
                    entry.req_id = Some(req_id_owned);
                    tracing::debug!(task_id = %task_id_clone, "WeCom: updated card entry with req_id");
                } else {
                    // Fallback: create new entry if not found (shouldn't happen normally)
                    tracing::warn!(task_id = %task_id_clone, "WeCom: card entry not found, creating new one without tool info");
                    entries.insert(
                        task_id_clone,
                        CardEventEntry {
                            req_id: Some(req_id_owned),
                            tool_name: "unknown".to_string(),
                            tool_display_name: "unknown".to_string(),
                            args_preview: "".to_string(),
                            created_at: std::time::Instant::now(),
                        },
                    );
                }
            });
        }

        tracing::info!(
            task_id,
            button_key,
            userid,
            ?chatid,
            %req_id,
            "WeCom: template card button clicked"
        );

        let decision = if button_key.starts_with("approve_allow_") {
            ApprovalDecision::AllowOnce
        } else if button_key.starts_with("approve_deny_") {
            ApprovalDecision::Deny
        } else {
            tracing::warn!(
                task_id,
                button_key,
                "WeCom: unknown button key, ignoring template card event"
            );
            return;
        };

        let reply_target = if chatid.is_some() && chatid.as_deref() != Some(&userid) {
            format!("group:{}", chatid.as_deref().unwrap_or(&userid))
        } else {
            format!("user:{userid}")
        };

        if let Some(url) = body.get("response_url").and_then(|u| u.as_str()) {
            let last_url = self.last_response_url.clone();
            let rt = reply_target.clone();
            let url = url.to_string();
            tokio::spawn(async move {
                let mut urls = last_url.write().await;
                urls.insert(rt, url);
            });
        }

        let approved = decision == ApprovalDecision::AllowOnce;
        let inbound_tx = inbound_tx.clone();
        let task_id_for_auth = task_id.clone();
        tokio::spawn(async move {
            let _ = inbound_tx
                .send(InboundMessage::Auth(AuthReply {
                    call_id: task_id_for_auth,
                    decision,
                }))
                .await;
        });

        let channel = Arc::clone(self);
        let task_id_for_card = task_id;
        tokio::spawn(async move {
            if let Err(e) = channel
                .update_approval_card(&task_id_for_card, approved)
                .await
            {
                tracing::warn!(request_id = %task_id_for_card, error = %e, "WeCom update_approval_card failed");
            }
        });
    }

    // ─── Connection loop ───

    /// Run a single WebSocket connection session.
    /// Returns `true` if reconnection should be attempted.
    async fn run_connection(
        self: &Arc<Self>,
        inbound_tx: mpsc::Sender<InboundMessage>,
        cancel: CancellationToken,
    ) -> bool {
        tracing::info!(url = %DEFAULT_WS_URL, "WeCom: connecting...");

        let connect_result = tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("WeCom: connection cancelled before connect");
                return false;
            }
            result = tokio_tungstenite::connect_async(DEFAULT_WS_URL) => result,
        };

        let (ws_stream, _) = match connect_result {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(err = %e, "WeCom: failed to connect");
                return true; // retry
            }
        };

        tracing::info!("WeCom: WebSocket connected, sending auth...");

        let (write, mut read) = ws_stream.split();
        {
            let mut w = self.ws_writer.write().await;
            *w = Some(write);
        }

        // Send auth frame.
        if let Err(e) = self.send_auth().await {
            tracing::error!(err = %e, "WeCom: failed to send auth");
            return true;
        }

        let mut authenticated = false;
        let mut missed_pong: u32 = 0;

        // Set up heartbeat interval — skip first tick.
        let mut hb_interval = tokio::time::interval(self.heartbeat_interval);
        hb_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        hb_interval.tick().await; // skip immediate first tick

        loop {
            tokio::select! {
                biased;

                _ = cancel.cancelled() => {
                    tracing::info!("WeCom: connection cancelled");
                    break;
                }

                // Heartbeat (only after authenticated)
                _ = hb_interval.tick(), if authenticated => {
                    if missed_pong >= MAX_MISSED_PONG {
                        tracing::warn!(
                            missed = missed_pong,
                            "WeCom: no heartbeat ACK for {} consecutive pings, connection dead",
                            missed_pong
                        );
                        break;
                    }
                    missed_pong += 1;
                    if let Err(e) = self.send_heartbeat().await {
                        tracing::error!(err = %e, "WeCom: failed to send heartbeat");
                        break;
                    }
                    tracing::debug!(missed_pong, "WeCom: heartbeat sent");
                }

                // Incoming WebSocket messages
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            match serde_json::from_str::<WsFrame>(&text) {
                                Ok(frame) => {
                                    self.handle_frame(
                                            frame,
                                            &mut authenticated,
                                            &mut missed_pong,
                                            &inbound_tx,
                                        )
                                        .await;
                                }
                                Err(e) => {
                                    tracing::warn!(err = %e, "WeCom: failed to parse frame");
                                }
                            }
                        }
                        Some(Ok(Message::Ping(data))) => {
                            tracing::trace!("WeCom: received WS ping, sending pong");
                            let mut w = self.ws_writer.write().await;
                            if let Some(ws) = w.as_mut() {
                                let _ = ws.send(Message::Pong(data)).await;
                            }
                        }
                        Some(Ok(Message::Close(close_frame))) => {
                            let reason = close_frame
                                .as_ref()
                                .map(|cf| format!("code={} reason={}", cf.code, cf.reason))
                                .unwrap_or_else(|| "no reason".to_string());
                            tracing::warn!(%reason, "WeCom: WebSocket closed by server");
                            break;
                        }
                        Some(Ok(_)) => {
                            // Binary, Pong, Frame — ignore
                        }
                        Some(Err(e)) => {
                            tracing::warn!(err = %e, "WeCom: WebSocket read error");
                            break;
                        }
                        None => {
                            tracing::info!("WeCom: WebSocket stream ended");
                            break;
                        }
                    }
                }
            }
        }

        // Cleanup
        {
            let mut w = self.ws_writer.write().await;
            *w = None;
        }
        // Clear pending ACKs
        {
            let mut pending = self.pending_acks.write().await;
            for (_id, ack) in pending.drain() {
                let _ = ack.tx.send(Err("connection closed".to_string()));
            }
        }

        true // reconnect
    }
}

/// 解析上层 (`channels/mod.rs::run_im_subscriber`) 传给 `update_draft` /
/// `finalize_draft` 的字符串。约定的三种形态见枚举注释。
///
/// 解析策略：宽松匹配 `</think>`，对 think 段、body 段两侧空白做 trim。
/// 上层 `final_text` 几乎不可能含字面 `</think>`；即便含，第一个出现的
/// `</think>` 一定是上层自己拼接的闭合标签，不影响切分。
#[derive(Debug)]
enum DraftParts<'a> {
    /// `<think>\n{think}` —— `update_draft` 阶段（不闭合）。
    ThinkOnly { think: &'a str },
    /// `<think>\n{think}\n</think>\n\n{body}` —— `finalize_draft` 阶段。
    ThinkAndBody { think: &'a str, body: &'a str },
    /// 纯正文（无思考），`finalize_draft` 在 accumulated 为空时直接发 `final_text`。
    BodyOnly { body: &'a str },
}

fn parse_draft_text(text: &str) -> DraftParts<'_> {
    let trimmed = text.trim_start();
    if let Some(after_open) = trimmed
        .strip_prefix("<think>\n")
        .or_else(|| trimmed.strip_prefix("<think>"))
    {
        if let Some(close_pos) = after_open.find("</think>") {
            let think = after_open[..close_pos].trim();
            let body = after_open[close_pos + "</think>".len()..].trim();
            DraftParts::ThinkAndBody { think, body }
        } else {
            DraftParts::ThinkOnly {
                think: after_open.trim(),
            }
        }
    } else {
        DraftParts::BodyOnly {
            body: text.trim(),
        }
    }
}

#[async_trait]
impl ImChannel for WeComChannel {
    async fn on_session_event(self: Arc<Self>, event: &moray_session::SessionEvent) {
        if let Some(out) = self.session_outbound.read().await.as_ref() {
            out.on_session_event(event).await;
        }
    }

    async fn run(
        self: Arc<Self>,
        cancel: CancellationToken,
    ) -> Result<ChannelRun, ChannelError> {
        self.init_outbound().await;
        let (inbound_tx, inbound_rx) = mpsc::channel(256);
        let self_run = Arc::clone(&self);
        let listener = tokio::spawn(async move {
            self_run.run_listener(inbound_tx, cancel).await
        });
        Ok(ChannelRun {
            receiver: inbound_rx,
            listener,
        })
    }
}

impl WeComChannel {
    async fn run_listener(
        self: Arc<Self>,
        inbound_tx: mpsc::Sender<InboundMessage>,
        cancel: CancellationToken,
    ) -> Result<(), ChannelError> {
        let mut attempt: u32 = 0;
        let base_delay_ms: u64 = 1000;

        loop {
            if cancel.is_cancelled() {
                break;
            }
            let should_reconnect = self.run_connection(inbound_tx.clone(), cancel.clone()).await;

            if !should_reconnect {
                tracing::info!("WeCom: clean disconnect, not reconnecting");
                break;
            }

            attempt += 1;
            if self.max_reconnect_attempts > 0 && attempt > self.max_reconnect_attempts {
                return Err(ChannelError::MaxReconnect {
                    limit: self.max_reconnect_attempts,
                });
            }

            let delay_ms = std::cmp::min(
                base_delay_ms * 2u64.saturating_pow(attempt.saturating_sub(1)),
                RECONNECT_MAX_DELAY_MS,
            );
            tracing::info!(attempt, delay_ms, "WeCom: reconnecting in {}ms...", delay_ms);
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_millis(delay_ms)) => {}
            }
        }

        Ok(())
    }
}

impl WeComChannel {
    pub async fn update_draft(
        &self,
        _recipient: &str,
        message_id: &str,
        text: &str,
        _reason: &str,
    ) -> Result<Option<String>, ChannelError> {
        // 空文本直接忽略：避免把空字符串塞进 draft 渲染流程，
        // 进而向企微发出空 reply 覆盖已显示内容。
        if text.is_empty() {
            return Ok(None);
        }
        // message_id = "req_id|stream_id"
        let parts: Vec<&str> = message_id.splitn(2, '|').collect();
        if parts.len() != 2 {
            return Ok(None);
        }
        let (req_id, stream_id) = (parts[0], parts[1]);

        // 上层约定：update_draft 阶段发 ThinkOnly（`<think>\n{思考}`，不闭合）。
        let display_text = match parse_draft_text(text) {
            DraftParts::ThinkOnly { think } => format!("<think>\n{think}"),
            DraftParts::ThinkAndBody { think, body } => {
                format!("<think>\n{think}\n</think>\n\n{body}")
            }
            DraftParts::BodyOnly { body } => body.to_string(),
        };
        if display_text.is_empty() {
            return Ok(None);
        }

        // 标记 draft 已发过内容（cancel_draft 据此决定是否发"已取消"）。
        // draft 已被并发 cancel/finalize 删除时静默忽略。
        {
            let mut drafts = self.active_drafts.write().await;
            match drafts.get_mut(req_id) {
                Some(d) => d.has_content = true,
                None => return Ok(None),
            }
        }

        // WeCom stream 协议是全量替换：每次发完整字符串覆盖前端显示。
        self.reply_stream(req_id, stream_id, &display_text, false).await?;
        Ok(None)
    }
    pub async fn finalize_draft(
        &self,
        recipient: &str,
        message_id: &str,
        text: &str,
    ) -> Result<(), ChannelError> {
        let parts: Vec<&str> = message_id.splitn(2, '|').collect();
        if parts.len() != 2 {
            return Ok(());
        }
        let (req_id, stream_id) = (parts[0], parts[1]);
        tracing::trace!("finalize_draft: input={}", text.replace('\n', ""));

        // 删除 draft 记录（cancel_draft 之后并发到达时静默丢弃）。
        {
            let mut drafts = self.active_drafts.write().await;
            drafts.remove(req_id);
        }

        let (think_block, body_text) = match parse_draft_text(text) {
            DraftParts::ThinkOnly { think } => {
                if think.is_empty() {
                    (String::new(), String::new())
                } else {
                    (
                        format!("<think>\n{think}\n</think>"),
                        String::new(),
                    )
                }
            }
            DraftParts::ThinkAndBody { think, body } => {
                let think_block = if think.is_empty() {
                    String::new()
                } else {
                    format!("<think>\n{think}\n</think>")
                };
                (think_block, body.to_string())
            }
            DraftParts::BodyOnly { body } => {
                if body.is_empty() {
                    (String::new(), String::new())
                } else {
                    (
                        format!("<think>\n{body}\n</think>"),
                        body.to_string(),
                    )
                }
            }
        };

        let response_url = {
            let mut urls = self.last_response_url.write().await;
            urls.remove(recipient)
        };

        // 流式气泡只承载思考块；正文始终独立成 card（与有 tool/审批路径一致）。
        if !think_block.is_empty() {
            let _ = self
                .reply_stream(req_id, stream_id, &think_block, true)
                .await;
        } else if !body_text.is_empty() {
            let _ = self.reply_stream(req_id, stream_id, "", true).await;
        }

        if !body_text.is_empty() {
            self.send_final_body_message(req_id, recipient, response_url, &body_text)
                .await?;
        }

        Ok(())
    }

    /// 正文以独立消息下发（审批卡下方或流式思考 card 下方）。
    async fn send_final_body_message(
        &self,
        req_id: &str,
        recipient: &str,
        response_url: Option<String>,
        body_text: &str,
    ) -> Result<(), ChannelError> {
        let chatid = recipient
            .strip_prefix("user:")
            .or_else(|| recipient.strip_prefix("group:"))
            .unwrap_or(recipient);
        let mut sent = false;
        if let Some(url) = response_url {
            if self.send_via_response_url(&url, body_text).await.is_ok() {
                sent = true;
            } else {
                tracing::warn!(req_id, "WeCom: response_url send failed, falling back");
            }
        }
        if !sent {
            let reply = WeComReplyRoute::new(recipient.to_string(), Some(req_id.to_string()));
            if self.deliver_plain_text(&reply, body_text).await.is_err() {
                if let Err(e) = self.send_proactive_message(chatid, body_text).await {
                    tracing::warn!(req_id, "WeCom: failed to send body as new message: {e}");
                }
            }
        }
        Ok(())
    }

    pub async fn cancel_draft(&self, _recipient: &str, message_id: &str) -> Result<(), ChannelError> {
        let parts: Vec<&str> = message_id.splitn(2, '|').collect();
        if parts.len() != 2 {
            return Ok(());
        }
        let (req_id, stream_id) = (parts[0], parts[1]);

        let draft = {
            let mut drafts = self.active_drafts.write().await;
            drafts.remove(req_id)
        };

        // Only finish stream if content was sent (otherwise WeCom doesn't know about it)
        if let Some(d) = draft {
            if d.has_content {
                self.reply_stream(req_id, stream_id, &t!("wecom-cancelled"), true)
                    .await?;
            }
        }

        Ok(())
    }
}

// ───────────────────── Media Download Types & Helpers ─────────────────────

/// A pending media download task collected during message parsing.
struct MediaDownloadTask {
    url: String,
    /// If present, the media is AES-encrypted and needs decryption.
    aes_key: Option<String>,
    is_image: bool,
    /// Placeholder string in the content that will be replaced with the local path.
    placeholder: String,
}

/// Download media (optionally decrypt) and save to the given directory.
///
/// Returns the final save path on success.
async fn download_and_save_media(
    task: &MediaDownloadTask,
    save_dir: &std::path::Path,
) -> Result<PathBuf, ChannelError> {
    // 1. Download
    let client = reqwest::Client::new();
    let resp = client
        .get(&task.url)
        .timeout(Duration::from_secs(60))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(ChannelError::op(format!(
            "Media download failed: HTTP {} - {}",
            resp.status(),
            task.url
        )));
    }

    let raw_bytes = resp.bytes().await?;

    if raw_bytes.is_empty() {
        return Err(ChannelError::op(format!(
            "Downloaded empty file from: {}",
            task.url
        )));
    }

    // 2. Decrypt if needed
    let data = if let Some(ref aes_key) = task.aes_key {
        decrypt_wecom(&raw_bytes, aes_key)?
    } else {
        raw_bytes.to_vec()
    };

    // 3. Determine extension from magic bytes
    let ext = ext_from_magic_bytes(&data);

    // 4. Determine filename: try extracting from URL, fallback to UUID
    let base_name =
        extract_filename_from_url(&task.url).unwrap_or_else(|| Uuid::new_v4().to_string());

    let save_path = save_dir.join(format!("{}.{}", base_name, ext));

    // 5. Write to disk
    tokio::fs::write(&save_path, &data).await.map_err(|e| {
        ChannelError::op(format!(
            "Failed to write media to {}: {e}",
            save_path.display()
        ))
    })?;

    Ok(save_path)
}

/// Extract a filename (without extension) from a URL path.
///
/// E.g. `https://xxx.com/path/to/photo.jpg?q-sign=xxx` → `"photo"`
fn extract_filename_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let path = parsed.path();
    let last_segment = path.rsplit('/').next()?;
    // Remove extension if present (we'll use magic bytes for the real extension)
    let name = last_segment.split('.').next()?;
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

// ───────────────────── WeCom AES Decryption ─────────────────────

/// Decrypt WeCom encrypted file using AES-256-CBC.
///
/// According to WeCom official SDK:
/// - Algorithm: AES-256-CBC
/// - Key: Base64-encoded 32-byte key from message `aeskey` field
/// - IV: First 16 bytes of the decoded key
/// - Padding: PKCS#7, padded to 32-byte blocks (not standard 16-byte)
fn decrypt_wecom(encrypted_data: &[u8], aeskey_base64: &str) -> Result<Vec<u8>, ChannelError> {
    use aes::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
    type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

    // WeCom may omit base64 padding '=' characters, so we need to add them back
    let mut aeskey_padded = aeskey_base64.to_string();
    let padding_needed = (4 - aeskey_padded.len() % 4) % 4;
    if padding_needed > 0 {
        aeskey_padded.push_str(&"=".repeat(padding_needed));
    }

    // 1. Decode Base64 aeskey
    let key = match BASE64_STANDARD.decode(&aeskey_padded) {
        Ok(k) => k,
        Err(e) => {
            tracing::error!(
                "[wecom] base64 decode failed: err={}, aeskey_original={}, aeskey_padded={}",
                e,
                aeskey_base64,
                aeskey_padded
            );
            return Err(ChannelError::op(format!("Invalid base64 aeskey: {e}")));
        }
    };

    if key.len() != 32 {
        return Err(ChannelError::op(format!(
            "WeCom aeskey must be 32 bytes, got {}",
            key.len()
        )));
    }

    // 2. IV is the first 16 bytes of the key
    let iv = &key[..16];

    // 3. AES-256-CBC decrypt
    let mut buffer = encrypted_data.to_vec();
    let decryptor = Aes256CbcDec::new_from_slices(&key, iv).map_err(|e| {
        ChannelError::op(format!("Failed to create AES decryptor: {e:?}"))
    })?;

    let decrypted = decryptor
        .decrypt_padded_mut::<NoPadding>(&mut buffer)
        .map_err(|e| ChannelError::op(format!("AES decryption failed: {e:?}")))?;

    // 4. Manually remove PKCS#7 padding (32-byte block)
    if decrypted.is_empty() {
        return Err(ChannelError::op("Decrypted data is empty"));
    }

    let pad_len = decrypted[decrypted.len() - 1] as usize;
    if pad_len < 1 || pad_len > 32 || pad_len > decrypted.len() {
        return Err(ChannelError::op(format!(
            "Invalid PKCS#7 padding value: {pad_len}"
        )));
    }

    // Verify all padding bytes are consistent
    for i in (decrypted.len() - pad_len)..decrypted.len() {
        if decrypted[i] as usize != pad_len {
            return Err(ChannelError::op("Invalid PKCS#7 padding: bytes mismatch"));
        }
    }

    Ok(decrypted[..decrypted.len() - pad_len].to_vec())
}

// ───────────────────── File Type Detection ─────────────────────

/// Detect file extension from magic bytes.
fn ext_from_magic_bytes(data: &[u8]) -> &'static str {
    if data.len() < 8 {
        return "bin";
    }

    // JPEG: FF D8 FF
    if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return "jpg";
    }

    // PNG: 89 50 4E 47 0D 0A 1A 0A
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return "png";
    }

    // GIF: 47 49 46 38
    if data.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        return "gif";
    }

    // WebP: 52 49 46 46 ... 57 45 42 50
    if data.len() >= 12 && data.starts_with(&[0x52, 0x49, 0x46, 0x46]) && &data[8..12] == b"WEBP" {
        return "webp";
    }

    // BMP: 42 4D
    if data.starts_with(&[0x42, 0x4D]) {
        return "bmp";
    }

    // PDF: 25 50 44 46 (%PDF)
    if data.starts_with(&[0x25, 0x50, 0x44, 0x46]) {
        return "pdf";
    }

    // MP4/MOV: ... ftyp
    if data.len() >= 8 && &data[4..8] == b"ftyp" {
        return "mp4";
    }

    // MP3: FF FB or ID3
    if (data[0] == 0xFF && data[1] == 0xFB) || data.starts_with(b"ID3") {
        return "mp3";
    }

    // OGG: 4F 67 67 53
    if data.starts_with(&[0x4F, 0x67, 0x67, 0x53]) {
        return "ogg";
    }

    // WAV: 52 49 46 46 ... 57 41 56 45
    if data.len() >= 12 && data.starts_with(&[0x52, 0x49, 0x46, 0x46]) && &data[8..12] == b"WAVE" {
        return "wav";
    }

    // ZIP/DOCX/XLSX: 50 4B 03 04
    if data.starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
        return "zip";
    }

    "bin"
}
