//! OpenAI-compatible adapter using [`async-openai`](https://crates.io/crates/async-openai).

use std::collections::HashMap;
use std::pin::Pin;
use std::time::Instant;

use async_openai::config::OpenAIConfig;
use async_openai::types::chat::{
    ChatChoiceStream, ChatCompletionMessageToolCall as AoMessageToolCall,
    ChatCompletionMessageToolCallChunk as AoMessageToolCallChunk,
    ChatCompletionMessageToolCalls as AoMessageToolCalls,
    ChatCompletionRequestAssistantMessage as AoRequestAssistantMessage,
    ChatCompletionRequestAssistantMessageContent as AoRequestAssistantMessageContent,
    ChatCompletionRequestMessage as AoRequestMessage,
    ChatCompletionRequestSystemMessage as AoRequestSystemMessage,
    ChatCompletionRequestSystemMessageContent as AoRequestSystemMessageContent,
    ChatCompletionRequestToolMessage as AoRequestToolMessage,
    ChatCompletionRequestToolMessageContent as AoRequestToolMessageContent,
    ChatCompletionRequestUserMessage as AoRequestUserMessage,
    ChatCompletionRequestUserMessageContent as AoRequestUserMessageContent,
    ChatCompletionStreamOptions, ChatCompletionTool as AoTool, ChatCompletionTools as AoTools,
    CompletionUsage, CreateChatCompletionRequest as AoCreateRequest,
    FinishReason as AoFinishReason, FunctionCall, FunctionObject, ReasoningEffort,
};
use async_openai::Client;
use async_trait::async_trait;
use futures::Stream;
use futures::StreamExt;
use moray_core::{
    ChatCompletion, ChatCompletionFinishReason, ChatCompletionRequestMessage,
    ChatCompletionResponseChunk, MorayError, ToolCallRequest, ToolManifest,
};
use serde::Serialize;
use serde_json::Value;
use tracing::{debug, info, instrument, warn};

const PROTOCOL_LOG_LIMIT: usize = 32_768;

/// OpenAI-compatible HTTP endpoint and model id for [`OpenAIChatCompletion`].
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub api_key: String,
    pub api_base: String,
    pub model: String,
    /// Opaque provider-specific options; interpreted by the completion adapter.
    pub extensions: Value,
}

impl Endpoint {
    pub fn new(
        api_key: impl Into<String>,
        api_base: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            api_key: api_key.into(),
            api_base: api_base.into(),
            model: model.into(),
            extensions: Value::Object(Default::default()),
        }
    }

    pub fn with_extensions(mut self, extensions: Value) -> Self {
        self.extensions = extensions;
        self
    }
}

pub struct OpenAIChatCompletion {
    client: Client<OpenAIConfig>,
    api_base: String,
    model: String,
    extensions: Value,
}

impl OpenAIChatCompletion {
    pub fn new(endpoint: Endpoint) -> Self {
        let api_base = endpoint.api_base.clone();
        let cfg = OpenAIConfig::new()
            .with_api_key(endpoint.api_key)
            .with_api_base(api_base.clone());
        Self {
            client: Client::with_config(cfg),
            api_base,
            model: endpoint.model,
            extensions: endpoint.extensions,
        }
    }

    pub fn from_config(cfg: OpenAIConfig, model: impl Into<String>) -> Self {
        use async_openai::config::Config;

        Self {
            client: Client::with_config(cfg.clone()),
            api_base: cfg.api_base().to_string(),
            model: model.into(),
            extensions: Value::Object(Default::default()),
        }
    }
}

fn to_request_messages(msgs: &[ChatCompletionRequestMessage]) -> Vec<AoRequestMessage> {
    msgs.iter()
        .map(|m| match m {
            ChatCompletionRequestMessage::System { content } => {
                AoRequestMessage::System(AoRequestSystemMessage {
                    content: AoRequestSystemMessageContent::Text(content.clone()),
                    name: None,
                })
            }
            ChatCompletionRequestMessage::User { content } => {
                AoRequestMessage::User(AoRequestUserMessage {
                    content: AoRequestUserMessageContent::Text(content.clone()),
                    name: None,
                })
            }
            ChatCompletionRequestMessage::Assistant {
                content,
                tool_calls,
            } => {
                let tool_calls = tool_calls.as_ref().map(|tc| {
                    tc.iter()
                        .map(|t| {
                            AoMessageToolCalls::Function(AoMessageToolCall {
                                id: t.call_id.clone(),
                                function: FunctionCall {
                                    name: t.name.clone(),
                                    arguments: serialize_tool_call_args(&t.arguments),
                                },
                            })
                        })
                        .collect()
                });
                AoRequestMessage::Assistant(AoRequestAssistantMessage {
                    content: if content.is_empty() && tool_calls.is_some() {
                        None
                    } else {
                        Some(AoRequestAssistantMessageContent::Text(content.clone()))
                    },
                    tool_calls,
                    ..Default::default()
                })
            }
            ChatCompletionRequestMessage::Tool {
                content,
                call_id: tool_call_id,
            } => AoRequestMessage::Tool(AoRequestToolMessage {
                content: AoRequestToolMessageContent::Text(content.clone()),
                tool_call_id: tool_call_id.clone(),
            }),
        })
        .collect()
}

#[derive(Default)]
struct MessageRoleCounts {
    system: usize,
    user: usize,
    assistant: usize,
    tool: usize,
}

#[derive(Default, Serialize)]
struct LlmToolCallProtocol {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default, Serialize)]
struct LlmResponseProtocol {
    #[serde(skip_serializing_if = "Option::is_none")]
    completion_id: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    refusal: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<LlmToolCallProtocol>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<CompletionUsage>,
    chunk_count: usize,
}

fn protocol_json(value: &impl Serialize) -> String {
    protocol_json_with_limit(value, PROTOCOL_LOG_LIMIT)
}

fn protocol_json_with_limit(value: &impl Serialize, limit: usize) -> String {
    match serde_json::to_string(value) {
        Ok(json) => truncate_protocol_payload(&json, limit),
        Err(error) => format!("<serialize error: {error}>"),
    }
}

fn truncate_protocol_payload(payload: &str, limit: usize) -> String {
    if payload.len() <= limit {
        return payload.to_string();
    }
    let mut end = limit.min(payload.len());
    while end > 0 && !payload.is_char_boundary(end) {
        end -= 1;
    }
    let total = payload.len();
    format!("{}... [truncated, total {total} bytes]", &payload[..end])
}

fn log_llm_request_protocol(model: &str, api_base: &str, wire_request: &AoCreateRequest) {
    debug!(
        model = %model,
        api_base = %api_base,
        request = %protocol_json(wire_request),
        "llm request protocol"
    );
}

fn count_message_roles(messages: &[ChatCompletionRequestMessage]) -> MessageRoleCounts {
    let mut counts = MessageRoleCounts::default();
    for message in messages {
        match message {
            ChatCompletionRequestMessage::System { .. } => counts.system += 1,
            ChatCompletionRequestMessage::User { .. } => counts.user += 1,
            ChatCompletionRequestMessage::Assistant { .. } => counts.assistant += 1,
            ChatCompletionRequestMessage::Tool { .. } => counts.tool += 1,
        }
    }
    counts
}

fn map_tools(tools: &[ToolManifest]) -> Option<Vec<AoTools>> {
    if tools.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for t in tools {
        let params: Option<serde_json::Value> = serde_json::from_str(&t.parameters).ok();
        out.push(AoTools::Function(AoTool {
            function: FunctionObject {
                name: t.name.clone(),
                description: Some(t.description.clone()),
                parameters: params,
                strict: None,
            },
        }));
    }
    Some(out)
}

fn finalize_completion_reason(
    fr: Option<&AoFinishReason>,
    mut refusal_buf: String,
) -> ChatCompletionFinishReason {
    let has_refusal = !refusal_buf.is_empty();
    match fr {
        Some(AoFinishReason::ToolCalls) | Some(AoFinishReason::FunctionCall) => {
            ChatCompletionFinishReason::Stop
        }
        Some(AoFinishReason::Length) => ChatCompletionFinishReason::Length,
        Some(AoFinishReason::ContentFilter) => ChatCompletionFinishReason::Refusal {
            reason: if has_refusal {
                Some(std::mem::take(&mut refusal_buf))
            } else {
                None
            },
        },
        Some(AoFinishReason::Stop) if has_refusal => ChatCompletionFinishReason::Refusal {
            reason: Some(std::mem::take(&mut refusal_buf)),
        },
        Some(AoFinishReason::Stop) => ChatCompletionFinishReason::Stop,
        None if has_refusal => ChatCompletionFinishReason::Refusal {
            reason: Some(std::mem::take(&mut refusal_buf)),
        },
        None => ChatCompletionFinishReason::Stop,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParsedTextChunk {
    Text(String),
    Think(String),
    ThinkDone,
}

/// Incremental parser for thinking sections delimited by known open/close tags.
struct ThinkTagStreamParser {
    buffer: String,
    in_think: bool,
    /// When true, stream starts in think mode until a close tag (HY3 without open tag).
    implicit_think: bool,
    saw_close: bool,
}

impl ThinkTagStreamParser {
    const THINK_OPEN: &'static str = concat!("<", "think", ">");
    const THINK_CLOSE: &'static str = concat!("<", "/", "think", ">");
    const OPEN_TAGS: &'static [&'static str] =
        &["<think>", Self::THINK_OPEN];
    const CLOSE_TAGS: &'static [&'static str] =
        &["</think>", Self::THINK_CLOSE];

    fn new() -> Self {
        Self {
            buffer: String::new(),
            in_think: false,
            implicit_think: false,
            saw_close: false,
        }
    }

    fn enter_implicit_think(&mut self) {
        if self.saw_close {
            return;
        }
        self.implicit_think = true;
        self.in_think = true;
    }

    fn push(&mut self, delta: &str) -> Vec<ParsedTextChunk> {
        self.buffer.push_str(delta);
        let mut out = Vec::new();

        loop {
            if self.in_think {
                if let Some((idx, close_tag)) = find_earliest_tag(self.buffer.as_str(), Self::CLOSE_TAGS)
                {
                    if idx > 0 {
                        let think = self.buffer[..idx].to_string();
                        out.push(ParsedTextChunk::Think(think));
                    }
                    self.buffer.drain(..idx + close_tag.len());
                    out.push(ParsedTextChunk::ThinkDone);
                    self.in_think = false;
                    self.saw_close = true;
                    continue;
                }

                let keep = trailing_partial_len_multi(&self.buffer, Self::CLOSE_TAGS);
                let flush_len = self.buffer.len().saturating_sub(keep);
                if flush_len > 0 {
                    let think = self.buffer[..flush_len].to_string();
                    self.buffer.drain(..flush_len);
                    out.push(ParsedTextChunk::Think(think));
                }
                break;
            }

            if let Some((idx, open_tag)) = find_earliest_tag(self.buffer.as_str(), Self::OPEN_TAGS) {
                if idx > 0 {
                    let text = self.buffer[..idx].to_string();
                    out.push(ParsedTextChunk::Text(text));
                }
                self.buffer.drain(..idx + open_tag.len());
                self.in_think = true;
                continue;
            }

            let keep = trailing_partial_len_multi(&self.buffer, Self::OPEN_TAGS);
            let flush_len = self.buffer.len().saturating_sub(keep);
            if flush_len > 0 {
                let text = self.buffer[..flush_len].to_string();
                self.buffer.drain(..flush_len);
                out.push(ParsedTextChunk::Text(text));
            }
            break;
        }

        out
    }

    fn finish(mut self) -> Vec<ParsedTextChunk> {
        if self.buffer.is_empty() {
            return Vec::new();
        }
        if self.in_think {
            if self.implicit_think && !self.saw_close {
                vec![ParsedTextChunk::Text(std::mem::take(&mut self.buffer))]
            } else {
                vec![ParsedTextChunk::Think(std::mem::take(&mut self.buffer))]
            }
        } else {
            vec![ParsedTextChunk::Text(std::mem::take(&mut self.buffer))]
        }
    }
}

fn find_earliest_tag<'a>(haystack: &str, tags: &'a [&'static str]) -> Option<(usize, &'a str)> {
    tags.iter()
        .filter_map(|tag| haystack.find(tag).map(|idx| (idx, *tag)))
        .min_by_key(|(idx, _)| *idx)
}

fn trailing_partial_len_multi(haystack: &str, tags: &[&str]) -> usize {
    tags.iter()
        .map(|tag| trailing_partial_len(haystack, tag))
        .max()
        .unwrap_or(0)
}

fn trailing_partial_len(haystack: &str, tag: &str) -> usize {
    let max = std::cmp::min(haystack.len(), tag.len().saturating_sub(1));
    for len in (1..=max).rev() {
        if haystack.ends_with(&tag[..len]) {
            return len;
        }
    }
    0
}

fn parse_reasoning_effort(raw: &str) -> Option<ReasoningEffort> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "none" | "no_think" | "no-think" => None,
        "minimal" => Some(ReasoningEffort::Minimal),
        "low" => Some(ReasoningEffort::Low),
        "medium" => Some(ReasoningEffort::Medium),
        "high" => Some(ReasoningEffort::High),
        "xhigh" | "x-high" => Some(ReasoningEffort::Xhigh),
        _ => None,
    }
}

/// Read `reasoning_effort` from opaque endpoint extensions (HunYuan / vLLM / OpenAI o-series).
fn reasoning_effort_from_extensions(extensions: &Value) -> Option<String> {
    extensions
        .get("reasoning_effort")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn merge_vllm_reasoning_extra_body(req: &AoCreateRequest, effort: &str) -> Value {
    let mut body = serde_json::to_value(req).unwrap_or_else(|_| Value::Object(Default::default()));
    if let Some(obj) = body.as_object_mut() {
        obj.insert(
            "chat_template_kwargs".to_string(),
            serde_json::json!({ "reasoning_effort": effort }),
        );
    }
    body
}

fn extract_reasoning_content_delta(resp: &impl Serialize) -> Option<String> {
    let value = serde_json::to_value(resp).ok()?;
    let content = value
        .get("choices")?
        .as_array()?
        .first()?
        .get("delta")?
        .get("reasoning_content")?
        .as_str()?;
    if content.is_empty() {
        None
    } else {
        Some(content.to_string())
    }
}

fn yield_parsed_text_chunks(
    parsed: ParsedTextChunk,
    saw_any_text: &mut bool,
    text_chars: &mut usize,
) -> Option<ChatCompletionResponseChunk> {
    match parsed {
        ParsedTextChunk::Text(text) => {
            if text.is_empty() {
                return None;
            }
            *saw_any_text = true;
            *text_chars += text.chars().count();
            Some(ChatCompletionResponseChunk::TextBlock(text))
        }
        ParsedTextChunk::Think(think) => {
            if think.is_empty() {
                return None;
            }
            Some(ChatCompletionResponseChunk::Think(think))
        }
        ParsedTextChunk::ThinkDone => Some(ChatCompletionResponseChunk::ThinkDone),
    }
}

#[async_trait]
impl ChatCompletion for OpenAIChatCompletion {
    #[instrument(
        name = "completion.openai",
        level = "info",
        skip(self, messages, tools, stream)
    )]
    async fn completion(
        &self,
        messages: &[ChatCompletionRequestMessage],
        tools: &[ToolManifest],
        stream: bool,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<ChatCompletionResponseChunk, MorayError>> + Send>>,
        MorayError,
    > {
        let roles = count_message_roles(messages);
        let tool_names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();

        let reasoning_effort_raw = reasoning_effort_from_extensions(&self.extensions);
        let reasoning_effort = reasoning_effort_raw
            .as_deref()
            .and_then(parse_reasoning_effort);

        let req = AoCreateRequest {
            model: self.model.clone(),
            messages: to_request_messages(messages),
            stream: Some(stream),
            stream_options: stream.then_some(ChatCompletionStreamOptions {
                include_usage: Some(true),
                include_obfuscation: None,
            }),
            tools: map_tools(tools),
            reasoning_effort: reasoning_effort.clone(),
            ..Default::default()
        };

        if let Some(effort) = reasoning_effort_raw.as_deref() {
            let wire = merge_vllm_reasoning_extra_body(&req, effort);
            debug!(
                model = %self.model,
                api_base = %self.api_base,
                request = %protocol_json(&wire),
                "llm request protocol (with chat_template_kwargs)"
            );
        } else {
            log_llm_request_protocol(self.model.as_str(), self.api_base.as_str(), &req);
        }

        info!(
            model = %self.model,
            api_base = %self.api_base,
            message_count = messages.len(),
            system_messages = roles.system,
            user_messages = roles.user,
            assistant_messages = roles.assistant,
            tool_messages = roles.tool,
            tool_count = tools.len(),
            tools = %tool_names.join(","),
            stream = stream,
            reasoning_effort = ?reasoning_effort_raw,
            "llm request"
        );

        let started_at = Instant::now();
        let connect_started = Instant::now();
        let mut upstream = self.client.chat().create_stream(req).await.map_err(|e| {
            warn!(
                model = %self.model,
                api_base = %self.api_base,
                elapsed_ms = connect_started.elapsed().as_millis() as u64,
                error = %e,
                "llm request failed"
            );
            MorayError::Message(e.to_string())
        })?;
        let connect_ms = connect_started.elapsed().as_millis() as u64;
        info!(
            model = %self.model,
            connect_ms,
            "llm stream connected"
        );

        let model = self.model.clone();
        let out = async_stream::stream! {
            let mut tool_buf: HashMap<u32, (String, String, String)> = HashMap::new();
            let mut refusal_buf = String::new();
            let mut saw_any_text = false;
            let mut think_parser = ThinkTagStreamParser::new();
            let reasoning_tag_fallback = reasoning_effort.is_some();
            let mut saw_reasoning_content = false;
            let mut activated_implicit_think = false;
            let mut in_reasoning_stream = false;
            let mut completion_id: Option<String> = None;
            let mut first_chunk_ms: Option<u64> = None;
            let mut text_chars = 0usize;
            let mut usage: Option<CompletionUsage> = None;
            let mut wire_content = String::new();
            let mut chunk_count = 0usize;

            while let Some(item) = upstream.next().await {
                let resp = match item {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(
                            model = %model,
                            completion_id = completion_id.as_deref().unwrap_or(""),
                            elapsed_ms = started_at.elapsed().as_millis() as u64,
                            error = %e,
                            "llm stream error"
                        );
                        yield Err(MorayError::Message(e.to_string()));
                        return;
                    }
                };

                if completion_id.is_none() {
                    completion_id = Some(resp.id.clone());
                    first_chunk_ms = Some(started_at.elapsed().as_millis() as u64);
                    debug!(
                        completion_id = %resp.id,
                        first_chunk_ms = first_chunk_ms.unwrap_or(0),
                        "llm first chunk"
                    );
                }

                chunk_count += 1;
                debug!(
                    completion_id = %resp.id,
                    chunk_index = chunk_count,
                    chunk = %protocol_json(&resp),
                    "llm response chunk"
                );

                let reasoning_delta = extract_reasoning_content_delta(&resp);

                if let Some(chunk_usage) = resp.usage {
                    usage = Some(chunk_usage);
                }

                let Some(choice) = resp.choices.first() else {
                    debug!(
                        completion_id = completion_id.as_deref().unwrap_or(""),
                        "llm chunk had no choices"
                    );
                    continue;
                };

                let ChatChoiceStream { delta, finish_reason, .. } = choice;

                if let Some(reasoning) = reasoning_delta {
                    in_reasoning_stream = true;
                    saw_reasoning_content = true;
                    yield Ok(ChatCompletionResponseChunk::Think(reasoning));
                }

                if let Some(t) = &delta.content {
                    if !t.is_empty() {
                        if in_reasoning_stream {
                            in_reasoning_stream = false;
                            yield Ok(ChatCompletionResponseChunk::ThinkDone);
                        }
                        if reasoning_tag_fallback
                            && !saw_reasoning_content
                            && !activated_implicit_think
                        {
                            think_parser.enter_implicit_think();
                            activated_implicit_think = true;
                        }
                        wire_content.push_str(t);
                        for parsed in think_parser.push(t) {
                            if let Some(chunk) =
                                yield_parsed_text_chunks(parsed, &mut saw_any_text, &mut text_chars)
                            {
                                yield Ok(chunk);
                            }
                        }
                    }
                }

                if let Some(r) = &delta.refusal {
                    if !r.is_empty() {
                        refusal_buf.push_str(r);
                    }
                }

                if let Some(tcs) = &delta.tool_calls {
                    for tc in tcs {
                        merge_tool_chunk(&mut tool_buf, tc);
                    }
                }

                if finish_reason.is_some() {
                    if in_reasoning_stream {
                        yield Ok(ChatCompletionResponseChunk::ThinkDone);
                    }
                    for parsed in std::mem::replace(&mut think_parser, ThinkTagStreamParser::new()).finish() {
                        if let Some(chunk) =
                            yield_parsed_text_chunks(parsed, &mut saw_any_text, &mut text_chars)
                        {
                            yield Ok(chunk);
                        }
                    }
                    let refusal = std::mem::take(&mut refusal_buf);
                    let fr = finalize_completion_reason(finish_reason.as_ref(), refusal.clone());
                    let mut finalized: Vec<ToolCallRequest> = tool_buf
                        .into_iter()
                        .map(|(_, (call_id, name, arguments))| ToolCallRequest {
                            call_id,
                            name,
                            arguments: parse_tool_call_args(&arguments),
                        })
                        .filter(|t| !t.name.is_empty())
                        .collect();
                    finalized.sort_by(|a, b| a.call_id.cmp(&b.call_id));
                    let tools_empty = finalized.is_empty();
                    let response_protocol = build_response_protocol(
                        completion_id.clone(),
                        wire_content,
                        refusal,
                        &finalized,
                        finish_reason.as_ref(),
                        usage.clone(),
                        chunk_count,
                    );
                    if saw_any_text && !tools_empty {
                        yield Ok(ChatCompletionResponseChunk::TextDone);
                    }
                    for tc in finalized {
                        yield Ok(ChatCompletionResponseChunk::ToolCall(tc));
                    }
                    if saw_any_text && tools_empty {
                        yield Ok(ChatCompletionResponseChunk::TextDone);
                    }
                    yield Ok(ChatCompletionResponseChunk::Done { reason: fr.clone() });
                    log_llm_completed(
                        model.as_str(),
                        connect_ms,
                        first_chunk_ms,
                        started_at.elapsed().as_millis() as u64,
                        text_chars,
                        finish_reason.as_ref(),
                        &fr,
                        &response_protocol,
                    );
                    return;
                }
            }

            if in_reasoning_stream {
                yield Ok(ChatCompletionResponseChunk::ThinkDone);
            }
            for parsed in think_parser.finish() {
                if let Some(chunk) = yield_parsed_text_chunks(parsed, &mut saw_any_text, &mut text_chars) {
                    yield Ok(chunk);
                }
            }

            let fr = finalize_completion_reason(None, refusal_buf.clone());
            yield Ok(ChatCompletionResponseChunk::Done {
                reason: fr.clone(),
            });

            let response_protocol = build_response_protocol(
                completion_id.clone(),
                wire_content,
                refusal_buf,
                &[],
                None,
                usage.clone(),
                chunk_count,
            );
            log_llm_completed(
                model.as_str(),
                connect_ms,
                first_chunk_ms,
                started_at.elapsed().as_millis() as u64,
                text_chars,
                None,
                &fr,
                &response_protocol,
            );
            debug!(
                completion_id = completion_id.as_deref().unwrap_or(""),
                "llm stream ended without explicit finish_reason"
            );
        };

        Ok(Box::pin(out))
    }
}

fn build_response_protocol(
    completion_id: Option<String>,
    content: String,
    refusal: String,
    tool_calls: &[ToolCallRequest],
    finish_reason: Option<&AoFinishReason>,
    usage: Option<CompletionUsage>,
    chunk_count: usize,
) -> LlmResponseProtocol {
    LlmResponseProtocol {
        completion_id,
        content,
        refusal: (!refusal.is_empty()).then_some(refusal),
        tool_calls: tool_calls
            .iter()
            .map(|tool_call| LlmToolCallProtocol {
                id: tool_call.call_id.clone(),
                name: tool_call.name.clone(),
                arguments: serialize_tool_call_args(&tool_call.arguments),
            })
            .collect(),
        finish_reason: finish_reason.map(|reason| format!("{reason:?}")),
        usage,
        chunk_count,
    }
}

fn log_llm_completed(
    model: &str,
    connect_ms: u64,
    first_chunk_ms: Option<u64>,
    total_ms: u64,
    text_chars: usize,
    upstream_finish_reason: Option<&AoFinishReason>,
    finalized_reason: &ChatCompletionFinishReason,
    response_protocol: &LlmResponseProtocol,
) {
    let completion_id = response_protocol.completion_id.as_deref().unwrap_or("");
    let tool_call_count = response_protocol.tool_calls.len();
    let usage = response_protocol.usage.as_ref();

    info!(
        model = %model,
        completion_id = completion_id,
        connect_ms,
        first_chunk_ms = ?first_chunk_ms,
        total_ms,
        text_chars,
        tool_call_count,
        chunk_count = response_protocol.chunk_count,
        upstream_finish_reason = ?upstream_finish_reason,
        finalized_reason = ?finalized_reason,
        prompt_tokens = usage.map(|u| u.prompt_tokens),
        completion_tokens = usage.map(|u| u.completion_tokens),
        total_tokens = usage.map(|u| u.total_tokens),
        "llm completed"
    );

    debug!(
        model = %model,
        completion_id = completion_id,
        response = %protocol_json(response_protocol),
        "llm response protocol"
    );
}

fn parse_tool_call_args(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Object(serde_json::Map::new());
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn serialize_tool_call_args(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => "{}".to_string(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_string()),
    }
}

fn merge_tool_chunk(buf: &mut HashMap<u32, (String, String, String)>, tc: &AoMessageToolCallChunk) {
    let entry = buf.entry(tc.index).or_insert_with(|| {
        (
            tc.id.clone().unwrap_or_default(),
            tc.function
                .as_ref()
                .and_then(|f| f.name.clone())
                .unwrap_or_default(),
            String::new(),
        )
    });
    if let Some(id) = &tc.id {
        if !id.is_empty() {
            entry.0 = id.clone();
        }
    }
    if let Some(f) = &tc.function {
        if let Some(n) = &f.name {
            if !n.is_empty() {
                entry.1 = n.clone();
            }
        }
        if let Some(a) = &f.arguments {
            entry.2.push_str(a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_reasoning_content_delta, merge_vllm_reasoning_extra_body, parse_reasoning_effort,
        reasoning_effort_from_extensions, truncate_protocol_payload, AoCreateRequest,
        ParsedTextChunk, ReasoningEffort, ThinkTagStreamParser,
    };

    fn text(s: &str) -> ParsedTextChunk {
        ParsedTextChunk::Text(s.into())
    }
    fn think(s: &str) -> ParsedTextChunk {
        ParsedTextChunk::Think(s.into())
    }

    #[test]
    fn parser_streams_think_and_text_with_split_tags() {
        let mut p = ThinkTagStreamParser::new();
        assert_eq!(p.push("hello <thi"), vec![text("hello ")]);
        assert_eq!(p.push("nk>abc</th"), vec![think("abc")]);
        assert_eq!(
            p.push("ink> world"),
            vec![ParsedTextChunk::ThinkDone, text(" world")]
        );
        assert!(p.finish().is_empty());
    }

    #[test]
    fn parser_handles_open_tag_char_by_char() {
        let mut p = ThinkTagStreamParser::new();
        assert_eq!(p.push("<"), Vec::<ParsedTextChunk>::new());
        assert_eq!(p.push("thi"), Vec::<ParsedTextChunk>::new());
        assert_eq!(p.push("nk>"), Vec::<ParsedTextChunk>::new());
        assert_eq!(p.push("x"), vec![think("x")]);
        assert_eq!(p.push("</"), Vec::<ParsedTextChunk>::new());
        assert_eq!(p.push("think"), Vec::<ParsedTextChunk>::new());
        assert_eq!(p.push(">"), vec![ParsedTextChunk::ThinkDone]);
        assert!(p.finish().is_empty());
    }

    #[test]
    fn parser_handles_open_tag_and_content_in_same_delta() {
        let mut p = ThinkTagStreamParser::new();
        assert_eq!(p.push("<think>xx"), vec![think("xx")]);
        assert_eq!(p.push("</think>"), vec![ParsedTextChunk::ThinkDone]);
        assert!(p.finish().is_empty());
    }

    #[test]
    fn parser_handles_close_tag_split_across_many_deltas() {
        let mut p = ThinkTagStreamParser::new();
        assert_eq!(p.push("<think>abc"), vec![think("abc")]);
        assert_eq!(p.push("</"), Vec::<ParsedTextChunk>::new());
        assert_eq!(p.push("th"), Vec::<ParsedTextChunk>::new());
        assert_eq!(p.push("ink"), Vec::<ParsedTextChunk>::new());
        assert_eq!(p.push(">"), vec![ParsedTextChunk::ThinkDone]);
        assert!(p.finish().is_empty());
    }

    #[test]
    fn parser_flushes_partial_open_tag_as_text_on_finish() {
        let mut p = ThinkTagStreamParser::new();
        assert_eq!(p.push("abc<thi"), vec![text("abc")]);
        assert_eq!(p.finish(), vec![text("<thi")]);
    }

    #[test]
    fn parser_flushes_partial_close_tag_as_think_on_finish() {
        let mut p = ThinkTagStreamParser::new();
        assert_eq!(p.push("<think>abc</th"), vec![think("abc")]);
        assert_eq!(p.finish(), vec![think("</th")]);
    }

    #[test]
    fn parser_handles_multiple_think_sections_and_interleaved_text() {
        let mut p = ThinkTagStreamParser::new();
        assert_eq!(
            p.push("a<think>x</think>b<think>y"),
            vec![
                text("a"),
                think("x"),
                ParsedTextChunk::ThinkDone,
                text("b"),
                think("y")
            ]
        );
        assert_eq!(
            p.push("</think>c"),
            vec![ParsedTextChunk::ThinkDone, text("c")]
        );
        assert!(p.finish().is_empty());
    }

    #[test]
    fn parser_implicit_think_splits_on_close_tag_without_open_tag() {
        let mut p = ThinkTagStreamParser::new();
        p.enter_implicit_think();
        assert_eq!(p.push("reasoning"), vec![think("reasoning")]);
        assert_eq!(
            p.push("</think>answer"),
            vec![ParsedTextChunk::ThinkDone, text("answer")]
        );
        assert!(p.finish().is_empty());
    }

    #[test]
    fn parser_implicit_think_without_close_tag_finishes_as_text() {
        let mut p = ThinkTagStreamParser::new();
        p.enter_implicit_think();
        assert_eq!(p.push("plain reply"), vec![think("plain reply")]);
        assert!(p.finish().is_empty());
    }

    #[test]
    fn parser_implicit_think_splits_on_think_close_tag() {
        let mut p = ThinkTagStreamParser::new();
        p.enter_implicit_think();
        assert_eq!(p.push("reasoning"), vec![think("reasoning")]);
        assert_eq!(
            p.push(concat!("<", "/", "think", ">") ),
            vec![ParsedTextChunk::ThinkDone]
        );
        assert_eq!(p.push("answer"), vec![text("answer")]);
        assert!(p.finish().is_empty());
    }

    #[test]
    fn parser_treats_invalid_tag_as_plain_text() {
        let mut p = ThinkTagStreamParser::new();
        assert_eq!(p.push("<thinking>foo"), vec![text("<thinking>foo")]);
        assert!(p.finish().is_empty());
    }

    #[test]
    fn reasoning_effort_from_extensions_reads_string_field() {
        let ext = serde_json::json!({ "reasoning_effort": "high" });
        assert_eq!(
            reasoning_effort_from_extensions(&ext).as_deref(),
            Some("high")
        );
        assert!(reasoning_effort_from_extensions(&serde_json::json!({})).is_none());
    }

    #[test]
    fn parse_reasoning_effort_maps_hunyuan_values() {
        assert_eq!(parse_reasoning_effort("high"), Some(ReasoningEffort::High));
        assert_eq!(parse_reasoning_effort("no_think"), None);
        assert_eq!(parse_reasoning_effort("unknown"), None);
    }

    #[test]
    fn merge_vllm_reasoning_extra_body_injects_chat_template_kwargs() {
        let req = AoCreateRequest {
            model: "hy3-preview".to_string(),
            messages: vec![],
            ..Default::default()
        };
        let body = merge_vllm_reasoning_extra_body(&req, "high");
        assert_eq!(
            body["chat_template_kwargs"]["reasoning_effort"].as_str(),
            Some("high")
        );
    }

    #[test]
    fn extract_reasoning_content_delta_reads_stream_json() {
        let json = serde_json::json!({
            "choices": [{
                "delta": { "reasoning_content": "step one" }
            }]
        });
        assert_eq!(
            extract_reasoning_content_delta(&json).as_deref(),
            Some("step one")
        );
    }

    #[test]
    fn truncate_protocol_payload_respects_utf8_char_boundary() {
        let payload = format!("{{\"msg\":\"{}\"}}", "你好");
        let limit = payload.len() - 1;
        let truncated = truncate_protocol_payload(&payload, limit);
        assert!(truncated.ends_with(" bytes]"));
        assert!(
            std::str::from_utf8(truncated.split("... [truncated").next().unwrap().as_bytes())
                .is_ok()
        );
    }
}
