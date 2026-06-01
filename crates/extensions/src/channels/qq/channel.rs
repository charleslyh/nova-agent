use crate::channels::qq::approval::parse_approval_command;
use crate::channels::qq::reply::QqReplyRoute;
use moray_channels::{AuthReply, ChannelError, ChannelRun, ImChannel, InboundMessage, UserMessage};
use crate::channels::qq::outbound::QqSessionOutbound;
use super::QQEnvironment;
use async_trait::async_trait;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;
use tokio_tungstenite::tungstenite::Message;

const QQ_API_BASE: &str = "https://api.sgroup.qq.com";
const QQ_SANDBOX_API_BASE: &str = "https://sandbox.api.sgroup.qq.com";
const QQ_AUTH_URL: &str = "https://bots.qq.com/app/getAppAccessToken";
const C2C_REPLY_TTL: Duration = Duration::from_secs(60 * 60);
const MESSAGE_REPLY_LIMIT: u32 = 5;
const TRACKER_CLEANUP_THRESHOLD: usize = 10_000;
const USER_INTERACTION_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Debug, Clone)]
struct UserInteraction {
    last_msg_id: String,
    msg_received_at: Instant,
    reply_count: u32,
    /// Monotonic counter for `msg_seq` sent to QQ API for this user's
    /// current passive-reply window.  Incremented on every successful
    /// passive reply regardless of caller (`send` or `send_scheduled_message`).
    msg_seq_counter: u32,
    last_interaction_at: SystemTime,
}

#[derive(Debug, Default)]
struct InteractionTracker {
    records: HashMap<String, UserInteraction>,
}

impl InteractionTracker {
    fn record_interaction(&mut self, user_openid: &str, msg_id: &str) {
        let user_openid = user_openid.trim();
        let msg_id = msg_id.trim();
        if user_openid.is_empty() || msg_id.is_empty() {
            return;
        }

        self.records.insert(
            user_openid.to_string(),
            UserInteraction {
                last_msg_id: msg_id.to_string(),
                msg_received_at: Instant::now(),
                reply_count: 0,
                msg_seq_counter: 0,
                last_interaction_at: SystemTime::now(),
            },
        );
        self.cleanup_expired();
    }

    /// Atomically increment both the reply count and the `msg_seq` counter
    /// for the given user.  Returns the new `msg_seq` value (1-based) that
    /// should be sent to the QQ API, or `None` if the limit has been reached
    /// or the record has expired.
    fn next_msg_seq(&mut self, user_openid: &str) -> Option<u32> {
        let record = self.records.get_mut(user_openid)?;
        if record.last_msg_id.trim().is_empty()
            || record.msg_received_at.elapsed() >= C2C_REPLY_TTL
            || record.reply_count >= MESSAGE_REPLY_LIMIT
        {
            return None;
        }
        record.reply_count += 1;
        record.msg_seq_counter += 1;
        Some(record.msg_seq_counter)
    }

    fn can_use_passive_reply(&self, user_openid: &str) -> Option<String> {
        let record = self.records.get(user_openid)?;
        if record.last_msg_id.trim().is_empty()
            || record.msg_received_at.elapsed() >= C2C_REPLY_TTL
            || record.reply_count >= MESSAGE_REPLY_LIMIT
        {
            return None;
        }
        Some(record.last_msg_id.clone())
    }

    fn can_use_recall_message(&self, user_openid: &str) -> bool {
        self.records
            .get(user_openid)
            .and_then(|record| record.last_interaction_at.elapsed().ok())
            .map(|elapsed| elapsed < USER_INTERACTION_TTL)
            .unwrap_or(false)
    }

    fn cleanup_expired(&mut self) {
        if self.records.len() <= TRACKER_CLEANUP_THRESHOLD {
            return;
        }

        self.records.retain(|_, record| {
            record
                .last_interaction_at
                .elapsed()
                .map(|elapsed| elapsed < USER_INTERACTION_TTL)
                .unwrap_or(false)
        });
    }
}

static INTERACTION_TRACKER: OnceLock<Arc<RwLock<InteractionTracker>>> = OnceLock::new();

fn get_interaction_tracker() -> Arc<RwLock<InteractionTracker>> {
    INTERACTION_TRACKER
        .get_or_init(|| Arc::new(RwLock::new(InteractionTracker::default())))
        .clone()
}

#[derive(Debug, Default)]
struct SessionState {
    session_id: Option<String>,
    last_seq: i64,
}

fn ensure_https(url: &str) -> Result<(), ChannelError> {
    let parsed =
        reqwest::Url::parse(url).map_err(|e| ChannelError::op(format!("Invalid URL '{url}': {e}")))?;
    if parsed.scheme() != "https" {
        return Err(ChannelError::op(
            "Refusing to transmit sensitive data over non-HTTPS URL: URL scheme must be https",
        ));
    }
    Ok(())
}

fn is_remote_media_url(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.starts_with("https://") || trimmed.starts_with("http://")
}

fn is_data_image_uri(target: &str) -> bool {
    let lower = target.trim().to_ascii_lowercase();
    lower.starts_with("data:image/") && lower.contains(";base64,")
}

fn is_image_filename(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".gif")
        || lower.ends_with(".webp")
        || lower.ends_with(".bmp")
        || lower.ends_with(".heic")
        || lower.ends_with(".heif")
        || lower.ends_with(".svg")
}

fn is_file_attachment(content_type: &str, filename: &str) -> bool {
    let lower_ct = content_type.to_ascii_lowercase();
    let lower_fn = filename.to_ascii_lowercase();
    // QQ supports: pdf, doc, txt
    lower_ct == "file"
        || lower_ct.contains("pdf")
        || lower_ct.contains("msword")
        || lower_ct.contains("document")
        || lower_ct.contains("text/plain")
        || lower_fn.ends_with(".pdf")
        || lower_fn.ends_with(".doc")
        || lower_fn.ends_with(".docx")
        || lower_fn.ends_with(".txt")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OutgoingImageTarget {
    RemoteUrl(String),
    LocalPath(String),
    DataUri(String),
}

impl OutgoingImageTarget {
    fn display_target(&self) -> &str {
        match self {
            Self::RemoteUrl(url) | Self::LocalPath(url) | Self::DataUri(url) => url,
        }
    }

    fn is_inline_data(&self) -> bool {
        matches!(self, Self::DataUri(_))
    }
}

fn extract_attachment_marker(attachment: &serde_json::Value) -> Option<String> {
    let url = attachment.get("url").and_then(|u| u.as_str())?.trim();
    if url.is_empty() {
        return None;
    }

    let content_type = attachment
        .get("content_type")
        .and_then(|ct| ct.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let filename = attachment
        .get("filename")
        .and_then(|f| f.as_str())
        .unwrap_or("");

    let is_image = content_type.starts_with("image/") || is_image_filename(filename);
    if is_image {
        // 如果有 content_type，同时输出 MIME 类型，方便 agent 判断图片格式
        if !content_type.is_empty() {
            return Some(t!("qq-image-upload-mime", url = url, content_type = &content_type));
        }
        return Some(t!("qq-image-upload", url = url));
    }

    let is_file = is_file_attachment(&content_type, filename);
    if is_file {
        let display_name = if filename.is_empty() { "file" } else { filename };
        return Some(format!("[FILE:{display_name}:{url}]"));
    }

    None
}

fn parse_image_marker_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let marker = trimmed.strip_prefix("[IMAGE:")?.strip_suffix(']')?.trim();
    if marker.is_empty() {
        return None;
    }
    Some(marker)
}

fn parse_outgoing_image_target(
    candidate: &str,
    allow_extensionless_remote_url: bool,
) -> Option<OutgoingImageTarget> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return None;
    }

    let normalized = trimmed.trim_matches(|c| matches!(c, '`' | '"' | '\''));
    let normalized = normalized.strip_prefix("file://").unwrap_or(normalized);
    if normalized.is_empty() {
        return None;
    }

    if is_data_image_uri(normalized) {
        return Some(OutgoingImageTarget::DataUri(normalized.to_string()));
    }

    if is_remote_media_url(normalized) {
        if allow_extensionless_remote_url || is_image_filename(normalized) {
            return Some(OutgoingImageTarget::RemoteUrl(normalized.to_string()));
        }
        return None;
    }

    if !is_image_filename(normalized) {
        return None;
    }

    let path = Path::new(normalized);
    if !path.is_file() {
        return None;
    }

    Some(OutgoingImageTarget::LocalPath(normalized.to_string()))
}

fn parse_outgoing_content(content: &str) -> (String, Vec<OutgoingImageTarget>) {
    let mut passthrough_lines = Vec::new();
    let mut image_targets = Vec::new();

    for line in content.lines() {
        if let Some(marker_target) = parse_image_marker_line(line) {
            if let Some(parsed) = parse_outgoing_image_target(marker_target, true) {
                image_targets.push(parsed);
                continue;
            }
        }

        if let Some(parsed) = parse_outgoing_image_target(line, false) {
            if matches!(
                parsed,
                OutgoingImageTarget::LocalPath(_) | OutgoingImageTarget::DataUri(_)
            ) {
                image_targets.push(parsed);
                continue;
            }
        }

        passthrough_lines.push(line);
    }

    (
        passthrough_lines.join("\n").trim().to_string(),
        image_targets,
    )
}

fn decode_data_image_payload(data_uri: &str) -> Result<String, ChannelError> {
    let trimmed = data_uri.trim();
    let (header, payload) = trimmed
        .split_once(',')
        .ok_or_else(|| ChannelError::op("invalid data URI: missing comma separator"))?;

    let lower_header = header.to_ascii_lowercase();
    if !lower_header.starts_with("data:image/") {
        return Err(ChannelError::op(format!(
            "unsupported data URI mime (expected image/*): {header}"
        )));
    }
    if !lower_header.contains(";base64") {
        return Err(ChannelError::op(format!(
            "unsupported data URI encoding (expected base64): {header}"
        )));
    }

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|e| ChannelError::op(format!("invalid data URI base64 payload: {e}")))?;
    if decoded.is_empty() {
        return Err(ChannelError::op("image payload is empty"));
    }

    Ok(base64::engine::general_purpose::STANDARD.encode(decoded))
}

fn compose_message_content(payload: &serde_json::Value) -> Option<String> {
    let text = payload
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim();

    let attachment_markers: Vec<String> = payload
        .get("attachments")
        .and_then(|a| a.as_array())
        .map(|attachments| {
            attachments
                .iter()
                .filter_map(extract_attachment_marker)
                .collect()
        })
        .unwrap_or_default();

    if text.is_empty() && attachment_markers.is_empty() {
        return None;
    }


    if text.is_empty() {
        return Some(attachment_markers.join("\n"));
    }

    if attachment_markers.is_empty() {
        return Some(text.to_string());
    }

    Some(format!("{text}\n\n{}", attachment_markers.join("\n")))
}

fn parse_face_tags(text: &str) -> String {
    let re = regex::Regex::new(r#"<faceType=\d+,faceId="[^"]*",ext="([^"]*)">"#)
        .expect("valid QQ face tag regex");

    re.replace_all(text, |caps: &regex::Captures| {
        caps.get(1)
            .and_then(|ext| {
                base64::engine::general_purpose::STANDARD
                    .decode(ext.as_str())
                    .ok()
            })
            .and_then(|decoded| String::from_utf8(decoded).ok())
            .and_then(|json_str| serde_json::from_str::<serde_json::Value>(&json_str).ok())
            .and_then(|parsed| {
                parsed
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .map(|text| t!("qq-emoji", text = text))
            })
            .unwrap_or_else(|| caps[0].to_string())
    })
    .to_string()
}

/// Parsed QQ user message before supervisor routing.
pub(crate) struct ParsedInboundMessage {
    pub reply_target: String,
    pub content: String,
    pub msg_id: String,
}

fn extract_message_id(payload: &serde_json::Value) -> &str {
    payload
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| payload.get("msg_id").and_then(Value::as_str))
        .unwrap_or("")
}

fn apply_passive_reply_fields(body: &mut Map<String, Value>, msg_id: Option<&str>, msg_seq: u64) {
    if let Some(msg_id) = msg_id {
        body.insert("msg_id".to_string(), Value::String(msg_id.to_string()));
        body.insert("msg_seq".to_string(), Value::from(msg_seq));
    }
}

fn build_text_message_body(content: &str, msg_id: Option<&str>, msg_seq: u64) -> Option<Value> {
    let text = content.trim();
    if text.is_empty() {
        return None;
    }

    let mut body = Map::new();
    body.insert("content".to_string(), Value::String(text.to_string()));
    body.insert("msg_type".to_string(), Value::from(0));
    apply_passive_reply_fields(&mut body, msg_id, msg_seq);

    Some(Value::Object(body))
}

fn build_media_message_body(file_info: &str, msg_id: Option<&str>, msg_seq: u64) -> Value {
    let mut body = Map::new();
    body.insert("content".to_string(), Value::String(" ".to_string()));
    body.insert("msg_type".to_string(), Value::from(7));
    body.insert("media".to_string(), json!({ "file_info": file_info }));
    apply_passive_reply_fields(&mut body, msg_id, msg_seq);
    Value::Object(body)
}

fn resolve_send_endpoints(api_base: &str, recipient: &str) -> (String, String) {
    if let Some(group_id) = recipient.strip_prefix("group:") {
        (
            format!("{api_base}/v2/groups/{group_id}/messages"),
            format!("{api_base}/v2/groups/{group_id}/files"),
        )
    } else {
        let raw_uid = recipient.strip_prefix("user:").unwrap_or(recipient);
        let user_id: String = raw_uid
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        (
            format!("{api_base}/v2/users/{user_id}/messages"),
            format!("{api_base}/v2/users/{user_id}/files"),
        )
    }
}

/// Deduplication set capacity — evict half of entries when full.
const DEDUP_CAPACITY: usize = 10_000;

/// QQ Official Bot channel — OAuth2 + WebSocket gateway (no HTTP webhook).
pub struct QQChannel {
    app_id: String,
    app_secret: String,
    environment: QQEnvironment,
    allowed_users: Vec<String>,
    /// Cached access token + expiry timestamp.
    token_cache: Arc<RwLock<Option<(String, u64)>>>,
    /// Message deduplication set.
    dedup: Arc<RwLock<HashSet<String>>>,
    /// Session state for Resume support across reconnects.
    session_state: Arc<RwLock<SessionState>>,
    session_outbound: Arc<RwLock<Option<QqSessionOutbound>>>,
}

impl QQChannel {
    pub fn new(
        app_id: String,
        app_secret: String,
        allowed_users: Vec<String>,
    ) -> Self {
        Self::new_with_environment(
            app_id,
            app_secret,
            allowed_users,
            QQEnvironment::Production,
        )
    }

    pub fn new_with_environment(
        app_id: String,
        app_secret: String,
        allowed_users: Vec<String>,
        environment: QQEnvironment,
    ) -> Self {
        Self {
            app_id,
            app_secret,
            environment,
            allowed_users,
            token_cache: Arc::new(RwLock::new(None)),
            dedup: Arc::new(RwLock::new(HashSet::new())),
            session_state: Arc::new(RwLock::new(SessionState::default())),
            session_outbound: Arc::new(RwLock::new(None)),
        }
    }

    /// Send plain text using passive or proactive QQ delivery.
    pub(crate) async fn deliver_plain_text(
        &self,
        reply: &QqReplyRoute,
        content: &str,
    ) -> Result<(), ChannelError> {
        let passive_msg_id = reply
            .msg_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let initial_seq: u64 = if passive_msg_id.is_some() {
            if let Some(user_openid) = reply.recipient.strip_prefix("user:") {
                let tracker = get_interaction_tracker();
                let mut guard = tracker.write().await;
                let next = guard.next_msg_seq(user_openid);
                drop(guard);
                match next {
                    Some(seq) => u64::from(seq),
                    None => {
                        return self
                            .send_recipient_content(&reply.recipient, content, None, 1)
                            .await;
                    }
                }
            } else {
                1
            }
        } else {
            1
        };

        if passive_msg_id.is_some() {
            match self
                .send_recipient_content(
                    &reply.recipient,
                    content,
                    passive_msg_id,
                    initial_seq,
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(err) => {
                    tracing::warn!(
                        recipient = %reply.recipient,
                        passive_msg_id = ?passive_msg_id,
                        error = %err,
                        "QQ passive send failed; falling back to proactive"
                    );
                }
            }
        }

        self.send_recipient_content(&reply.recipient, content, None, 1)
            .await
    }

    async fn init_outbound(self: &Arc<Self>) {
        let mut slot = self.session_outbound.write().await;
        if slot.is_none() {
            *slot = Some(QqSessionOutbound::new(Arc::clone(self)));
        }
    }

    async fn dispatch_inbound_message(
        self: &Arc<Self>,
        event_type: &str,
        payload: &serde_json::Value,
        inbound_tx: &mpsc::Sender<InboundMessage>,
    ) -> Result<(), ChannelError> {
        let Some(parsed) = self.parse_dispatch_message_event(event_type, payload).await else {
            return Ok(());
        };

        let route = QqReplyRoute::new(
            parsed.reply_target.clone(),
            (!parsed.msg_id.is_empty()).then(|| parsed.msg_id.clone()),
        );

        if let Some(out) = self.session_outbound.write().await.as_mut() {
            out.set_reply_route(route.clone()).await;
        }

        if let Some(cmd) = parse_approval_command(&parsed.content) {
            let _ = inbound_tx
                .send(InboundMessage::Auth(AuthReply {
                    call_id: cmd.request_id,
                    decision: cmd.decision,
                }))
                .await;
            return Ok(());
        }

        let _ = inbound_tx
            .send(InboundMessage::User(UserMessage {
                content: parsed.content,
            }))
            .await;
        Ok(())
    }

    fn http_client(&self) -> reqwest::Client {
        reqwest::Client::new()
    }

    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    fn api_base(&self) -> &'static str {
        match self.environment {
            QQEnvironment::Production => QQ_API_BASE,
            QQEnvironment::Sandbox => QQ_SANDBOX_API_BASE,
        }
    }

    fn is_user_allowed(&self, user_id: &str) -> bool {
        self.allowed_users.iter().any(|u| u == "*" || u == user_id)
    }

    async fn parse_dispatch_message_event(
        &self,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Option<ParsedInboundMessage> {
        match event_type {
            "C2C_MESSAGE_CREATE" => {
                let msg_id = extract_message_id(payload);
                tracing::info!(msg_id, "QQ: received C2C_MESSAGE_CREATE event");
                if self.is_duplicate(msg_id).await {
                    tracing::info!(msg_id, "QQ: dropping duplicate C2C message");
                    return None;
                }

                let content = match compose_message_content(payload) {
                    Some(c) => parse_face_tags(&c),
                    None => {
                        tracing::info!(msg_id, "QQ: C2C message has no content, skipping");
                        return None;
                    }
                };
                let author_id = payload
                    .get("author")
                    .and_then(|a| a.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let user_openid = payload
                    .get("author")
                    .and_then(|a| a.get("user_openid"))
                    .and_then(Value::as_str)
                    .unwrap_or(author_id);

                if !self.is_user_allowed(user_openid) {
                    tracing::warn!(
                        "QQ: ignoring C2C message from unauthorized user: {user_openid}"
                    );
                    return None;
                }

                tracing::info!(
                    user_openid,
                    msg_id,
                    content_len = content.len(),
                    content = %content,
                    "QQ: C2C message accepted, dispatching"
                );

                if !msg_id.is_empty() {
                    let tracker = get_interaction_tracker();
                    tracker
                        .write()
                        .await
                        .record_interaction(user_openid, msg_id);
                }

                let chat_id = format!("user:{user_openid}");
                Some(ParsedInboundMessage {
                    reply_target: chat_id,
                    content,
                    msg_id: msg_id.to_string(),
                })
            }
            "GROUP_AT_MESSAGE_CREATE" => {
                let msg_id = extract_message_id(payload);
                tracing::info!(msg_id, "QQ: received GROUP_AT_MESSAGE_CREATE event");
                if self.is_duplicate(msg_id).await {
                    tracing::info!(msg_id, "QQ: dropping duplicate group message");
                    return None;
                }

                let content = match compose_message_content(payload) {
                    Some(c) => parse_face_tags(&c),
                    None => {
                        tracing::info!(msg_id, "QQ: group message has no content, skipping");
                        return None;
                    }
                };
                let author_id = payload
                    .get("author")
                    .and_then(|a| a.get("member_openid"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                if !self.is_user_allowed(author_id) {
                    tracing::warn!(
                        "QQ: ignoring group message from unauthorized user: {author_id}"
                    );
                    return None;
                }

                let group_openid = payload
                    .get("group_openid")
                    .and_then(Value::as_str)
                    .or_else(|| payload.get("group_id").and_then(Value::as_str))
                    .unwrap_or("unknown");

                tracing::info!(
                    author_id,
                    group_openid,
                    msg_id,
                    content_len = content.len(),
                    content = %content,
                    "QQ: group message accepted, dispatching"
                );

                let chat_id = format!("group:{group_openid}");
                Some(ParsedInboundMessage {
                    reply_target: chat_id,
                    content,
                    msg_id: msg_id.to_string(),
                })
            }
            _ => {
                tracing::debug!(event_type, "QQ: unhandled dispatch event type");
                None
            }
        }
    }

    async fn post_json(
        &self,
        token: &str,
        url: &str,
        body: &Value,
        op: &str,
    ) -> Result<(), ChannelError> {
        ensure_https(url)?;
        let parsed_url = reqwest::Url::parse(url)
            .map_err(|e| ChannelError::op(format!("Invalid URL '{url}' for QQ {op}: {e}")))?;

        let resp = self
            .http_client()
            .post(parsed_url)
            .header("Authorization", format!("QQBot {token}"))
            .json(body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            let sanitized = format!("{err}");
            return Err(ChannelError::op(format!(
                "QQ {op} failed ({status}): {sanitized}"
            )));
        }

        Ok(())
    }

    async fn upload_media_file_info(
        &self,
        token: &str,
        files_url: &str,
        media_url: &str,
    ) -> Result<String, ChannelError> {
        ensure_https(files_url)?;
        ensure_https(media_url)?;
        let parsed_files_url = reqwest::Url::parse(files_url)
            .map_err(|e| {
                ChannelError::op(format!("Invalid QQ files endpoint URL '{files_url}': {e}"))
            })?;

        let upload_body = json!({
            "file_type": 1,
            "url": media_url,
            "srv_send_msg": false
        });

        let resp = self
            .http_client()
            .post(parsed_files_url)
            .header("Authorization", format!("QQBot {token}"))
            .json(&upload_body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            let sanitized = format!("{err}");
            return Err(ChannelError::op(format!(
                "QQ upload media failed ({status}): {sanitized}"
            )));
        }

        let payload: Value = resp.json().await?;
        let file_info = payload
            .get("file_info")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ChannelError::op("QQ upload media response missing file_info"))?;

        Ok(file_info.to_string())
    }

    async fn upload_media_file_data(
        &self,
        token: &str,
        files_url: &str,
        file_data_base64: &str,
    ) -> Result<String, ChannelError> {
        ensure_https(files_url)?;
        let parsed_files_url = reqwest::Url::parse(files_url)
            .map_err(|e| {
                ChannelError::op(format!("Invalid QQ files endpoint URL '{files_url}': {e}"))
            })?;

        let upload_body = json!({
            "file_type": 1,
            "file_data": file_data_base64,
            "srv_send_msg": false
        });

        let resp = self
            .http_client()
            .post(parsed_files_url)
            .header("Authorization", format!("QQBot {token}"))
            .json(&upload_body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            let sanitized = format!("{err}");
            return Err(ChannelError::op(format!(
                "QQ upload media(file_data) failed ({status}): {sanitized}"
            )));
        }

        let payload: Value = resp.json().await?;
        let file_info = payload
            .get("file_info")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ChannelError::op("QQ upload media(file_data) response missing file_info")
            })?;

        Ok(file_info.to_string())
    }

    /// Fetch an access token from QQ's OAuth2 endpoint.
    async fn fetch_access_token(&self) -> Result<(String, u64), ChannelError> {
        let body = json!({
            "appId": self.app_id,
            "clientSecret": self.app_secret,
        });

        let resp = self
            .http_client()
            .post(QQ_AUTH_URL)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            let sanitized = format!("{err}");
            return Err(ChannelError::op(format!(
                "QQ token request failed ({status}): {sanitized}"
            )));
        }

        let data: serde_json::Value = resp.json().await?;
        let token = data
            .get("access_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| ChannelError::op("Missing access_token in QQ response"))?
            .to_string();

        let expires_in = data
            .get("expires_in")
            .and_then(|e| e.as_str())
            .and_then(|e| e.parse::<u64>().ok())
            .unwrap_or(7200);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Expire 60 seconds early to avoid edge cases
        let expiry = now + expires_in.saturating_sub(60);

        Ok((token, expiry))
    }

    /// Get a valid access token, refreshing if expired.
    async fn get_token(&self) -> Result<String, ChannelError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        {
            let cache = self.token_cache.read().await;
            if let Some((ref token, expiry)) = *cache {
                if now < expiry {
                    return Ok(token.clone());
                }
            }
        }

        let (token, expiry) = self.fetch_access_token().await?;
        {
            let mut cache = self.token_cache.write().await;
            *cache = Some((token.clone(), expiry));
        }
        Ok(token)
    }

    /// Get the WebSocket gateway URL.
    async fn get_gateway_url(&self, token: &str) -> Result<String, ChannelError> {
        let resp = self
            .http_client()
            .get(format!("{}/gateway", self.api_base()))
            .header("Authorization", format!("QQBot {token}"))
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            let sanitized = format!("{err}");
            return Err(ChannelError::op(format!(
                "QQ gateway request failed ({status}): {sanitized}"
            )));
        }

        let data: serde_json::Value = resp.json().await?;
        let url = data
            .get("url")
            .and_then(|u| u.as_str())
            .ok_or_else(|| ChannelError::op("Missing gateway URL in QQ response"))?
            .to_string();

        Ok(url)
    }

    /// Check and insert message ID for deduplication.
    async fn is_duplicate(&self, msg_id: &str) -> bool {
        if msg_id.is_empty() {
            return false;
        }

        let mut dedup = self.dedup.write().await;

        if dedup.contains(msg_id) {
            return true;
        }

        // Evict oldest half when at capacity
        if dedup.len() >= DEDUP_CAPACITY {
            let to_remove: Vec<String> = dedup.iter().take(DEDUP_CAPACITY / 2).cloned().collect();
            for key in to_remove {
                dedup.remove(&key);
            }
        }

        dedup.insert(msg_id.to_string());
        false
    }

    async fn send_recipient_content(
        &self,
        recipient: &str,
        content: &str,
        passive_msg_id: Option<&str>,
        initial_msg_seq: u64,
    ) -> Result<(), ChannelError> {
        let mut msg_seq = initial_msg_seq;
        let token = self.get_token().await?;
        let (message_url, files_url) = resolve_send_endpoints(self.api_base(), recipient);
        let (text_content, image_urls) = parse_outgoing_content(content);
        let image_len = image_urls.len();

        // Resolve user_openid for seq bookkeeping when sending multiple
        // parts (text + images) in a single passive reply.
        let user_openid_for_seq = passive_msg_id
            .and_then(|_| recipient.strip_prefix("user:"))
            .map(str::to_string);

        if let Some(body) = build_text_message_body(&text_content, passive_msg_id, msg_seq) {
            self.post_json(&token, &message_url, &body, "send message")
                .await?;
            // Only bump msg_seq when we are actually going to send another part
            // (e.g. an image) in the same passive reply. Otherwise we'll skip a
            // msg_seq value and QQ rejects subsequent passive messages.
            if passive_msg_id.is_some() && image_len > 0 {
                // Need next seq for subsequent parts; get it from the tracker
                if let Some(ref uid) = user_openid_for_seq {
                    let tracker = get_interaction_tracker();
                    let mut guard = tracker.write().await;
                    if let Some(next) = guard.next_msg_seq(uid) {
                        msg_seq = u64::from(next);
                    }
                    drop(guard);
                }
            }
        }

        for (idx, image_target) in image_urls.into_iter().enumerate() {
            let file_info = match &image_target {
                OutgoingImageTarget::RemoteUrl(image_url) => {
                    self.upload_media_file_info(&token, &files_url, image_url)
                        .await
                }
                OutgoingImageTarget::LocalPath(path) => match tokio::fs::read(path).await {
                    Ok(bytes) => {
                        if bytes.is_empty() {
                            Err(ChannelError::op(format!(
                                "QQ local image payload is empty: {path}"
                            )))
                        } else {
                            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                            self.upload_media_file_data(&token, &files_url, &encoded)
                                .await
                        }
                    }
                    Err(e) => Err(ChannelError::op(format!(
                        "QQ local image read failed ({path}): {e}"
                    ))),
                },
                OutgoingImageTarget::DataUri(data_uri) => match decode_data_image_payload(data_uri)
                {
                    Ok(encoded) => {
                        self.upload_media_file_data(&token, &files_url, &encoded)
                            .await
                    }
                    Err(err) => Err(err),
                },
            };

            let file_info = match file_info {
                Ok(file_info) => file_info,
                Err(err) => {
                    let upload_error = err;
                    tracing::warn!(
                        "QQ: failed to upload image target '{}': {upload_error}",
                        if image_target.is_inline_data() {
                            "[inline image data]"
                        } else {
                            image_target.display_target()
                        }
                    );
                    let fallback_text = if image_target.is_inline_data() {
                        "Image attachment upload failed".to_string()
                    } else {
                        format!("Image: {}", image_target.display_target())
                    };
                    if let Some(body) =
                        build_text_message_body(&fallback_text, passive_msg_id, msg_seq)
                    {
                        self.post_json(&token, &message_url, &body, "send message")
                            .await?;
                        if passive_msg_id.is_some() && idx + 1 < image_len {
                            if let Some(ref uid) = user_openid_for_seq {
                                let tracker = get_interaction_tracker();
                                let mut guard = tracker.write().await;
                                if let Some(next) = guard.next_msg_seq(uid) {
                                    msg_seq = u64::from(next);
                                }
                                drop(guard);
                            }
                        }
                    }
                    continue;
                }
            };

            let media_body = build_media_message_body(&file_info, passive_msg_id, msg_seq);
            self.post_json(&token, &message_url, &media_body, "send message")
                .await?;
            if passive_msg_id.is_some() && idx + 1 < image_len {
                if let Some(ref uid) = user_openid_for_seq {
                    let tracker = get_interaction_tracker();
                    let mut guard = tracker.write().await;
                    if let Some(next) = guard.next_msg_seq(uid) {
                        msg_seq = u64::from(next);
                    }
                    drop(guard);
                }
            }
        }

        Ok(())
    }

    pub async fn send_scheduled_message(
        &self,
        user_openid: &str,
        content: &str,
    ) -> Result<ScheduledMessageType, ChannelError> {
        let user_openid = user_openid.trim();
        if user_openid.is_empty() {
            return Err(ChannelError::op(
                "QQ delivery target must be in format 'user:OPENID'",
            ));
        }

        let tracker = get_interaction_tracker();
        let recipient = format!("user:{user_openid}");

        // Try passive reply: get msg_id + first seq atomically
        let passive_attempt = {
            let mut t = tracker.write().await;
            t.can_use_passive_reply(user_openid)
                .and_then(|msg_id| t.next_msg_seq(user_openid).map(|seq| (msg_id, seq)))
        };

        if let Some((msg_id, seq)) = passive_attempt {
            match self
                .send_recipient_content(&recipient, content, Some(&msg_id), u64::from(seq))
                .await
            {
                Ok(()) => {
                    return Ok(ScheduledMessageType::Passive);
                }
                Err(err) => {
                    tracing::warn!(
                        "QQ: passive scheduled delivery failed for user {}: {}",
                        user_openid,
                        err
                    );
                }
            }
        }

        if tracker.read().await.can_use_recall_message(user_openid) {
            match self
                .send_recipient_content(&recipient, content, None, 1)
                .await
            {
                Ok(()) => return Ok(ScheduledMessageType::Recall),
                Err(err) => {
                    tracing::warn!(
                        "QQ: recall scheduled delivery failed for user {}: {}",
                        user_openid,
                        err
                    );
                }
            }
        }

        self.send_recipient_content(&recipient, content, None, 1)
            .await?;
        Ok(ScheduledMessageType::Proactive)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledMessageType {
    Passive,
    Recall,
    Proactive,
}

impl std::fmt::Display for ScheduledMessageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Passive => write!(f, "passive"),
            Self::Recall => write!(f, "recall"),
            Self::Proactive => write!(f, "proactive"),
        }
    }
}

#[async_trait]
impl ImChannel for QQChannel {
    async fn on_session_event(self: Arc<Self>, event: &moray_session::SessionEvent) {
        if let Some(out) = self.session_outbound.write().await.as_mut() {
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

impl QQChannel {
    #[allow(clippy::too_many_lines)]
    async fn run_listener(
        self: Arc<Self>,
        inbound_tx: mpsc::Sender<InboundMessage>,
        cancel: CancellationToken,
    ) -> Result<(), ChannelError> {
        // Outer loop: handles server-initiated reconnects (op 7) and invalid
        // sessions (op 9) entirely within the QQ channel, per the QQ gateway
        // protocol spec.  Only true errors (network failures, auth errors)
        // bubble up to the supervisor.
        macro_rules! cancelable {
            ($future:expr) => {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        tracing::info!("QQ: gateway listener cancelled");
                        return Ok(());
                    }
                    result = $future => result?,
                }
            };
        }

        loop {
            tracing::info!("QQ: authenticating...");
            let token = cancelable!(self.get_token());

            tracing::info!("QQ: fetching gateway URL...");
            let gw_url = cancelable!(self.get_gateway_url(&token));

            tracing::info!("QQ: connecting to gateway WebSocket...");
            let (ws_stream, _) = cancelable!(tokio_tungstenite::connect_async(&gw_url));
            let (mut write, mut read) = ws_stream.split();

            // Read Hello (opcode 10)
            let hello_msg = tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("QQ: gateway listener cancelled");
                    return Ok(());
                }
                msg = read.next() => msg,
            };
            let hello = hello_msg
                .ok_or(ChannelError::op("QQ: no hello frame"))??;
            let hello_data: serde_json::Value = serde_json::from_str(&hello.to_string())?;
            let heartbeat_interval = hello_data
                .get("d")
                .and_then(|d| d.get("heartbeat_interval"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(41250);

            let existing_session_id = { self.session_state.read().await.session_id.clone() };
            if let Some(session_id) = existing_session_id {
                // Op 6: Resume — restore previous session after op 7 reconnect.
                let seq = self.session_state.read().await.last_seq;
                let resume = json!({
                    "op": 6,
                    "d": {
                        "token": format!("QQBot {token}"),
                        "session_id": session_id,
                        "seq": seq,
                    }
                });
                write.send(Message::Text(resume.to_string().into())).await?;
            } else {
                // Op 2: Identify — fresh login (first connect or after op 9).
                // Intents: PUBLIC_GUILD_MESSAGES (1<<30) | C2C_MESSAGE_CREATE & GROUP_AT_MESSAGE_CREATE (1<<25)
                let intents: u64 = (1 << 25) | (1 << 30);
                let identify = json!({
                    "op": 2,
                    "d": {
                        "token": format!("QQBot {token}"),
                        "intents": intents,
                        "properties": {
                            "os": "linux",
                            "browser": "zeroclaw",
                            "device": "zeroclaw",
                        }
                    }
                });
                write
                    .send(Message::Text(identify.to_string().into()))
                    .await?;
            }

            tracing::info!("QQ: connected to gateway");

            // Heartbeat interval timer - runs directly in the select! loop
            // to avoid channel backpressure issues that could delay heartbeats.
            let hb_duration = std::time::Duration::from_millis(heartbeat_interval);
            let mut hb_interval = tokio::time::interval(hb_duration);
            hb_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            // Skip the first immediate tick; we want to wait before the first heartbeat.
            hb_interval.tick().await;

            // Inner loop: process messages on the current WebSocket connection.
            // `break` exits to the outer loop for reconnect; `return` exits listen().
            let mut should_reconnect = false;
            let mut graceful_stop = false;
            loop {
                tokio::select! {
                    // Heartbeat has higher priority - use biased to check it first
                    biased;

                    _ = cancel.cancelled() => {
                        tracing::info!("QQ: gateway listener cancelled");
                        graceful_stop = true;
                        break;
                    }

                    _ = hb_interval.tick() => {
                        let seq = self.session_state.read().await.last_seq;
                        let d = if seq >= 0 { json!(seq) } else { json!(null) };
                        let hb = json!({"op": 1, "d": d});
                        tracing::debug!(seq, "QQ: sending heartbeat");
                        if write
                            .send(Message::Text(hb.to_string().into()))
                            .await
                            .is_err()
                        {
                            tracing::warn!("QQ: failed to send heartbeat");
                            break;
                        }
                    }
                    msg = read.next() => {
                        let msg = match msg {
                            Some(Ok(Message::Text(t))) => t,
                            Some(Ok(Message::Close(frame))) => {
                                if let Some(cf) = frame {
                                    tracing::warn!(
                                        code = %cf.code,
                                        reason = %cf.reason,
                                        "QQ: WebSocket closed by server"
                                    );
                                    // 4009 = session timeout, 4900-4913 = recoverable errors per QQ docs
                                    let code_u16: u16 = cf.code.into();
                                    if code_u16 == 4009 || (4900..=4913).contains(&code_u16) {
                                        should_reconnect = true;
                                    }
                                } else {
                                    tracing::warn!("QQ: WebSocket closed by server (no close frame)");
                                    should_reconnect = true;
                                }
                                break;
                            }
                            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => continue,
                            Some(Ok(other)) => {
                                tracing::debug!(?other, "QQ: ignoring non-text message");
                                continue;
                            }
                            Some(Err(e)) => {
                                tracing::warn!(%e, "QQ: WebSocket read error");
                                should_reconnect = true;
                                break;
                            }
                            None => {
                                tracing::warn!("QQ: WebSocket stream ended (server closed connection)");
                                should_reconnect = true;
                                break;
                            }
                        };

                        let event: serde_json::Value = match serde_json::from_str(msg.as_ref()) {
                            Ok(e) => e,
                            Err(_) => continue,
                        };

                        if let Some(s) = event.get("s").and_then(serde_json::Value::as_i64) {
                            self.session_state.write().await.last_seq = s;
                        }

                        let op = event.get("op").and_then(serde_json::Value::as_u64).unwrap_or(0);

                        match op {
                            // Server requests immediate heartbeat
                            1 => {
                                let seq = self.session_state.read().await.last_seq;
                                let d = if seq >= 0 { json!(seq) } else { json!(null) };
                                let hb = json!({"op": 1, "d": d});
                                if write
                                    .send(Message::Text(hb.to_string().into()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                                continue;
                            }
                            // Op 7: Reconnect — server asks us to reconnect.
                            // Keep session state so we Resume on the next iteration.
                            7 => {
                                tracing::warn!("QQ: received Reconnect (op 7), reconnecting immediately");
                                should_reconnect = true;
                                break;
                            }
                            // Op 9: Invalid Session — clear session and re-Identify.
                            9 => {
                                tracing::warn!("QQ: received Invalid Session (op 9), reconnecting with fresh session");
                                let mut state = self.session_state.write().await;
                                state.session_id = None;
                                state.last_seq = -1;
                                should_reconnect = true;
                                break;
                            }
                            // Op 11: Heartbeat ACK — server confirms heartbeat received.
                            11 => {
                                tracing::debug!("QQ: heartbeat ACK received");
                                continue;
                            }
                            _ => {}
                        }

                        // Only process dispatch events (op 0)
                        if op != 0 {
                            continue;
                        }

                        let event_type = event.get("t").and_then(|t| t.as_str()).unwrap_or("");
                        let d = match event.get("d") {
                            Some(d) => d,
                            None => continue,
                        };

                        match event_type {
                            "READY" => {
                                if let Some(session_id) = d.get("session_id").and_then(Value::as_str) {
                                    tracing::info!(session_id, "QQ: READY received, session established");
                                    self.session_state.write().await.session_id = Some(session_id.to_string());
                                }
                            }
                            "RESUMED" => {
                                tracing::info!("QQ: session resumed successfully");
                            }
                            _ => {
                                tracing::debug!(event_type, "QQ: dispatch event received");
                                if let Err(e) = self
                                    .dispatch_inbound_message(event_type, d, &inbound_tx)
                                    .await
                                {
                                    tracing::warn!(error = %e, "QQ inbound dispatch failed");
                                }
                            }
                        }
                    }
                }
            }

            if graceful_stop {
                return Ok(());
            }

            if !should_reconnect {
                // Connection dropped unexpectedly — let supervisor handle backoff.
                return Err(ChannelError::op("QQ WebSocket connection closed"));
            }
            // Otherwise loop back and reconnect immediately.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_app_id_accessor() {
        let ch = QQChannel::new("app".into(), "secret".into(), vec![]);
        assert_eq!(ch.app_id(), "app");
    }

    #[test]
    fn test_user_allowed_wildcard() {
        let ch = QQChannel::new("app".into(), "secret".into(), vec!["*".into()]);
        assert!(ch.is_user_allowed("anyone"));
    }

    #[test]
    fn test_user_allowed_specific() {
        let ch = QQChannel::new("app".into(), "secret".into(), vec!["user123".into()]);
        assert!(ch.is_user_allowed("user123"));
        assert!(!ch.is_user_allowed("other"));
    }

    #[test]
    fn test_user_denied_empty() {
        let ch = QQChannel::new("app".into(), "secret".into(), vec![]);
        assert!(!ch.is_user_allowed("anyone"));
    }

    #[tokio::test]
    async fn test_dedup() {
        let ch = QQChannel::new("app".into(), "secret".into(), vec![]);
        assert!(!ch.is_duplicate("msg1").await);
        assert!(ch.is_duplicate("msg1").await);
        assert!(!ch.is_duplicate("msg2").await);
    }

    #[tokio::test]
    async fn test_dedup_empty_id() {
        let ch = QQChannel::new("app".into(), "secret".into(), vec![]);
        // Empty IDs should never be considered duplicates
        assert!(!ch.is_duplicate("").await);
        assert!(!ch.is_duplicate("").await);
    }

    #[test]
    fn test_channel_data_serde() {
        let data = serde_json::json!({
            "app_id": "12345",
            "app_secret": "secret_abc",
            "allowed_users": ["user1"]
        });
        let app_id = data["app_id"].as_str().unwrap();
        let app_secret = data["app_secret"].as_str().unwrap();
        let users: Vec<&str> = data["allowed_users"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(app_id, "12345");
        assert_eq!(app_secret, "secret_abc");
        assert_eq!(users, vec!["user1"]);
    }

    #[test]
    fn test_resolve_send_endpoints_respects_selected_api_base() {
        let (group_messages, group_files) =
            resolve_send_endpoints(QQ_SANDBOX_API_BASE, "group:12345");
        assert_eq!(
            group_messages,
            "https://sandbox.api.sgroup.qq.com/v2/groups/12345/messages"
        );
        assert_eq!(
            group_files,
            "https://sandbox.api.sgroup.qq.com/v2/groups/12345/files"
        );

        let (user_messages, user_files) = resolve_send_endpoints(QQ_API_BASE, "user:abc_123");
        assert_eq!(
            user_messages,
            "https://api.sgroup.qq.com/v2/users/abc_123/messages"
        );
        assert_eq!(
            user_files,
            "https://api.sgroup.qq.com/v2/users/abc_123/files"
        );
    }

    #[test]
    fn test_compose_message_content_text_only() {
        let payload = json!({
            "content": "  hello world  "
        });

        assert_eq!(
            compose_message_content(&payload),
            Some("hello world".to_string())
        );
    }

    #[test]
    fn test_compose_message_content_attachment_only_image() {
        let payload = json!({
            "content": "   ",
            "attachments": [
                {
                    "content_type": "image/jpg",
                    "url": "https://cdn.example.com/a.jpg"
                }
            ]
        });

        assert_eq!(
            compose_message_content(&payload),
            Some("[用户上传图片是:https://cdn.example.com/a.jpg (image/jpg)]".to_string())
        );
    }

    #[test]
    fn test_compose_message_content_text_and_image_attachments() {
        let payload = json!({
            "content": "Here is an image",
            "attachments": [
                {
                    "content_type": "image/png",
                    "url": "https://cdn.example.com/a.png"
                },
                {
                    "filename": "b.jpeg",
                    "url": "https://cdn.example.com/b.jpeg"
                }
            ]
        });

        assert_eq!(
            compose_message_content(&payload),
            Some(
                "Here is an image\n\n[用户上传图片是:https://cdn.example.com/a.png (image/png)]\n[用户上传图片是:https://cdn.example.com/b.jpeg]"
                    .to_string()
            )
        );
    }

    #[test]
    fn test_compose_message_content_file_attachment() {
        let payload = json!({
            "content": "text",
            "attachments": [
                {
                    "content_type": "application/pdf",
                    "filename": "report.pdf",
                    "url": "https://cdn.example.com/a.pdf"
                }
            ]
        });

        assert_eq!(
            compose_message_content(&payload),
            Some("text\n\n[FILE:report.pdf:https://cdn.example.com/a.pdf]".to_string())
        );
    }

    #[test]
    fn test_compose_message_content_ignores_unsupported_attachments() {
        let payload = json!({
            "content": "text",
            "attachments": [
                {
                    "content_type": "video/mp4",
                    "url": "https://cdn.example.com/a.mp4"
                }
            ]
        });

        assert_eq!(compose_message_content(&payload), Some("text".to_string()));
    }

    #[test]
    fn test_compose_message_content_drops_empty_without_valid_attachments() {
        let payload = json!({
            "content": "   ",
            "attachments": [
                {
                    "content_type": "video/mp4",
                    "url": "https://cdn.example.com/a.mp4"
                },
                {
                    "content_type": "image/png",
                    "url": "   "
                }
            ]
        });

        assert_eq!(compose_message_content(&payload), None);
    }

    #[test]
    fn test_parse_outgoing_content_extracts_remote_image_markers() {
        let input = "hello\n[IMAGE:https://cdn.example.com/a.png]\n[IMAGE:http://cdn.example.com/b.jpg]\nbye";
        let (text, images) = parse_outgoing_content(input);

        assert_eq!(text, "hello\nbye");
        assert_eq!(
            images,
            vec![
                OutgoingImageTarget::RemoteUrl("https://cdn.example.com/a.png".to_string()),
                OutgoingImageTarget::RemoteUrl("http://cdn.example.com/b.jpg".to_string())
            ]
        );
    }

    #[test]
    fn test_parse_outgoing_content_accepts_marker_remote_url_without_extension() {
        let input = "hello\n[IMAGE:https://multimedia.nt.qq.com.cn/download?appid=1406]\nbye";
        let (text, images) = parse_outgoing_content(input);

        assert_eq!(text, "hello\nbye");
        assert_eq!(
            images,
            vec![OutgoingImageTarget::RemoteUrl(
                "https://multimedia.nt.qq.com.cn/download?appid=1406".to_string()
            )]
        );
    }

    #[test]
    fn test_parse_outgoing_content_keeps_non_remote_image_marker_as_text() {
        let input = "[IMAGE:/tmp/a.png]\nhello";
        let (text, images) = parse_outgoing_content(input);

        assert_eq!(text, "[IMAGE:/tmp/a.png]\nhello");
        assert!(images.is_empty());
    }

    #[test]
    fn test_parse_outgoing_content_extracts_existing_local_path_lines() {
        let temp = tempfile::tempdir().expect("temp dir");
        let local_path = temp.path().join("capture.png");
        std::fs::write(&local_path, b"png-bytes").expect("write local image");

        let input = format!("done\n{}\nnext", local_path.display());
        let (text, images) = parse_outgoing_content(&input);

        assert_eq!(text, "done\nnext");
        assert_eq!(
            images,
            vec![OutgoingImageTarget::LocalPath(
                local_path.display().to_string()
            )]
        );
    }

    #[test]
    fn test_parse_outgoing_content_extracts_data_uri_markers() {
        let input = "hello\n[IMAGE:data:image/png;base64,aGVsbG8=]\nbye";
        let (text, images) = parse_outgoing_content(input);

        assert_eq!(text, "hello\nbye");
        assert_eq!(
            images,
            vec![OutgoingImageTarget::DataUri(
                "data:image/png;base64,aGVsbG8=".to_string()
            )]
        );
    }

    #[test]
    fn test_build_text_message_body_with_passive_fields() {
        let body = build_text_message_body("hello", Some("msg-123"), 2).expect("text body");
        assert_eq!(
            body,
            json!({
                "content": "hello",
                "msg_type": 0,
                "msg_id": "msg-123",
                "msg_seq": 2
            })
        );
    }

    #[test]
    fn test_build_media_message_body_with_passive_fields() {
        let body = build_media_message_body("file-info-abc", Some("msg-123"), 3);
        assert_eq!(
            body,
            json!({
                "content": " ",
                "msg_type": 7,
                "media": {
                    "file_info": "file-info-abc"
                },
                "msg_id": "msg-123",
                "msg_seq": 3
            })
        );
    }

    #[test]
    fn test_parse_face_tags_decodes_embedded_payload() {
        let ext = base64::engine::general_purpose::STANDARD.encode(r#"{"text":"笑哭"}"#);
        let input = format!(r#"hello <faceType=1,faceId="14",ext="{}">"#, ext);
        assert_eq!(parse_face_tags(&input), format!("hello {}", t!("qq-emoji", text = "笑哭")));
    }

    #[test]
    fn test_interaction_tracker_enforces_limit_and_returns_seq() {
        let mut tracker = InteractionTracker::default();
        tracker.record_interaction("user1", "msg-1");
        for expected_seq in 1..=MESSAGE_REPLY_LIMIT {
            assert_eq!(tracker.next_msg_seq("user1"), Some(expected_seq));
        }
        // Limit reached
        assert_eq!(tracker.next_msg_seq("user1"), None);
        // can_use_passive_reply should also reflect the limit
        assert!(tracker.can_use_passive_reply("user1").is_none());
    }

    #[test]
    fn test_scheduled_message_type_display() {
        assert_eq!(ScheduledMessageType::Passive.to_string(), "passive");
        assert_eq!(ScheduledMessageType::Recall.to_string(), "recall");
        assert_eq!(ScheduledMessageType::Proactive.to_string(), "proactive");
    }
}
