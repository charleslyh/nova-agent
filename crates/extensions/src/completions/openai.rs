//! OpenAI-compatible adapter using [`async-openai`](https://crates.io/crates/async-openai).

use std::collections::HashMap;
use std::pin::Pin;

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
    ChatCompletionTool as AoTool, ChatCompletionTools as AoTools,
    CreateChatCompletionRequest as AoCreateRequest, FinishReason as AoFinishReason, FunctionCall,
    FunctionObject,
};
use async_openai::Client;
use async_trait::async_trait;
use futures::Stream;
use futures::StreamExt;
use moray_core::{
    ChatCompletion, ChatCompletionFinishReason, ChatCompletionRequestMessage,
    ChatCompletionResponseChunk, MorayError, ToolCallRequest, ToolManifest,
};
use tracing::{debug, info, instrument, warn};

/// OpenAI-compatible HTTP endpoint and model id for [`OpenAIChatCompletion`].
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub api_key: String,
    pub api_base: String,
    pub model: String,
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
        }
    }
}

pub struct OpenAIChatCompletion {
    client: Client<OpenAIConfig>,
    model: String,
}

impl OpenAIChatCompletion {
    pub fn new(endpoint: Endpoint) -> Self {
        let cfg = OpenAIConfig::new()
            .with_api_key(endpoint.api_key)
            .with_api_base(endpoint.api_base);
        Self::from_config(cfg, endpoint.model)
    }

    pub fn from_config(cfg: OpenAIConfig, model: impl Into<String>) -> Self {
        Self {
            client: Client::with_config(cfg),
            model: model.into(),
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
                                    arguments: t.arguments.clone(),
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

/// Incremental top-down parser for `<think>...</think>` sections over streamed deltas.
struct ThinkTagStreamParser {
    buffer: String,
    in_think: bool,
}

impl ThinkTagStreamParser {
    const OPEN_TAG: &'static str = "<think>";
    const CLOSE_TAG: &'static str = "</think>";

    fn new() -> Self {
        Self {
            buffer: String::new(),
            in_think: false,
        }
    }

    fn push(&mut self, delta: &str) -> Vec<ParsedTextChunk> {
        self.buffer.push_str(delta);
        let mut out = Vec::new();

        loop {
            if self.in_think {
                if let Some(idx) = self.buffer.find(Self::CLOSE_TAG) {
                    if idx > 0 {
                        let think = self.buffer[..idx].to_string();
                        out.push(ParsedTextChunk::Think(think));
                    }
                    self.buffer.drain(..idx + Self::CLOSE_TAG.len());
                    out.push(ParsedTextChunk::ThinkDone);
                    self.in_think = false;
                    continue;
                }

                let keep = trailing_partial_len(&self.buffer, Self::CLOSE_TAG);
                let flush_len = self.buffer.len().saturating_sub(keep);
                if flush_len > 0 {
                    let think = self.buffer[..flush_len].to_string();
                    self.buffer.drain(..flush_len);
                    out.push(ParsedTextChunk::Think(think));
                }
                break;
            }

            if let Some(idx) = self.buffer.find(Self::OPEN_TAG) {
                if idx > 0 {
                    let text = self.buffer[..idx].to_string();
                    out.push(ParsedTextChunk::Text(text));
                }
                self.buffer.drain(..idx + Self::OPEN_TAG.len());
                self.in_think = true;
                continue;
            }

            let keep = trailing_partial_len(&self.buffer, Self::OPEN_TAG);
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
            vec![ParsedTextChunk::Think(std::mem::take(&mut self.buffer))]
        } else {
            vec![ParsedTextChunk::Text(std::mem::take(&mut self.buffer))]
        }
    }
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
        info!(
            model = %self.model,
            message_count = messages.len(),
            tool_count = tools.len(),
            stream = stream,
            "started"
        );

        let req = AoCreateRequest {
            model: self.model.clone(),
            messages: to_request_messages(messages),
            stream: Some(stream),
            tools: map_tools(tools),
            ..Default::default()
        };

        let mut upstream = self.client.chat().create_stream(req).await.map_err(|e| {
            warn!(error = %e, "create stream failed");
            MorayError::Message(e.to_string())
        })?;

        let out = async_stream::stream! {
            let mut tool_buf: HashMap<u32, (String, String, String)> = HashMap::new();
            let mut refusal_buf = String::new();
            let mut saw_any_text = false;
            let mut think_parser = ThinkTagStreamParser::new();

            while let Some(item) = upstream.next().await {
                let resp = match item {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(error = %e, "stream item error");
                        yield Err(MorayError::Message(e.to_string()));
                        return;
                    }
                };

                let Some(choice) = resp.choices.first() else {
                    debug!("response had no choices");
                    continue;
                };

                let ChatChoiceStream { delta, finish_reason, .. } = choice;

                if let Some(t) = &delta.content {
                    if !t.is_empty() {
                        for parsed in think_parser.push(t) {
                            match parsed {
                                ParsedTextChunk::Text(text) => {
                                    if !text.is_empty() {
                                        saw_any_text = true;
                                        yield Ok(ChatCompletionResponseChunk::TextBlock(text));
                                    }
                                }
                                ParsedTextChunk::Think(think) => {
                                    if !think.is_empty() {
                                        yield Ok(ChatCompletionResponseChunk::Think(think));
                                    }
                                }
                                ParsedTextChunk::ThinkDone => {
                                    yield Ok(ChatCompletionResponseChunk::ThinkDone);
                                }
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
                    for parsed in std::mem::replace(&mut think_parser, ThinkTagStreamParser::new()).finish() {
                        match parsed {
                            ParsedTextChunk::Text(text) => {
                                if !text.is_empty() {
                                    saw_any_text = true;
                                    yield Ok(ChatCompletionResponseChunk::TextBlock(text));
                                }
                            }
                            ParsedTextChunk::Think(think) => {
                                if !think.is_empty() {
                                    yield Ok(ChatCompletionResponseChunk::Think(think));
                                }
                            }
                            ParsedTextChunk::ThinkDone => {
                                yield Ok(ChatCompletionResponseChunk::ThinkDone);
                            }
                        }
                    }
                    let fr = finalize_completion_reason(
                        finish_reason.as_ref(),
                        std::mem::take(&mut refusal_buf),
                    );
                    let mut finalized: Vec<ToolCallRequest> = tool_buf
                        .into_iter()
                        .map(|(_, (call_id, name, arguments))| ToolCallRequest {
                            call_id,
                            name,
                            arguments,
                        })
                        .filter(|t| !t.name.is_empty())
                        .collect();
                    finalized.sort_by(|a, b| a.call_id.cmp(&b.call_id));
                    let nb_finalized = finalized.len();
                    let tools_empty = finalized.is_empty();
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

                    info!(
                        finish_reason = ?finish_reason,
                        finalized_reason = ?fr,
                        saw_any_text,
                        tool_call_count = nb_finalized,
                        "finalized"
                    );
                    return;
                }
            }

            for parsed in think_parser.finish() {
                match parsed {
                    ParsedTextChunk::Text(text) => {
                        if !text.is_empty() {
                            yield Ok(ChatCompletionResponseChunk::TextBlock(text));
                        }
                    }
                    ParsedTextChunk::Think(think) => {
                        if !think.is_empty() {
                            yield Ok(ChatCompletionResponseChunk::Think(think));
                        }
                    }
                    ParsedTextChunk::ThinkDone => {
                        yield Ok(ChatCompletionResponseChunk::ThinkDone);
                    }
                }
            }

            yield Ok(ChatCompletionResponseChunk::Done {
                reason: finalize_completion_reason(None, refusal_buf),
            });

            debug!("stream ended without explicit finish_reason");
        };

        Ok(Box::pin(out))
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
    use super::{ParsedTextChunk, ThinkTagStreamParser};

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
    fn parser_treats_invalid_tag_as_plain_text() {
        let mut p = ThinkTagStreamParser::new();
        assert_eq!(p.push("<thinking>foo"), vec![text("<thinking>foo")]);
        assert!(p.finish().is_empty());
    }
}
