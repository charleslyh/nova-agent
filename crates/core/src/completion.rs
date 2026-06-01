use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::types::MorayError;
use crate::types::{ToolCallRequest, ToolManifest};

/// One message in the completion context.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "role", rename_all = "lowercase"))]
pub enum ChatCompletionRequestMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: String,
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        tool_calls: Option<Vec<ToolCallRequest>>,
    },
    Tool {
        content: String,
        call_id: String,
    },
}

/// Why the model stopped generating this completion round.
///
/// This is a **semantic** summary for moray, not a wire-format mirror. In particular, a model round that
/// ends because tools should run is already expressed by [`ChatCompletionResponseChunk::ToolCall`] items
/// before [`ChatCompletionResponseChunk::Done`]; adapters map wire `finish_reason: "tool_calls"` to [`Stop`](Self::Stop).
///
/// Policy / safety outcomes use [`Refusal`](Self::Refusal): `reason: None` when only filter/truncation is
/// indicated, `reason: Some` when explicit refusal wording was merged by the adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ChatCompletionFinishReason {
    /// Natural end of the completion stream for this round (includes OpenAI `stop` and `tool_calls` - use chunks for tools).
    Stop,
    /// Hit the requested or model `max_tokens` / output budget.
    Length,
    /// Provider declined or withheld output: optional human-readable explanation.
    Refusal {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        reason: Option<String>,
    },
}

/// One item from a [`ChatCompletion::completion`] stream.
///
/// Canonical order for one model round:
///
/// 1. zero or more [`ChatCompletionResponseChunk::TextBlock`] and/or [`ChatCompletionResponseChunk::Think`]
/// 2. optional [`ChatCompletionResponseChunk::ThinkDone`] after the last [`ChatCompletionResponseChunk::Think`]
/// 3. optional [`ChatCompletionResponseChunk::TextDone`] after the last [`ChatCompletionResponseChunk::TextBlock`]
///    when the round also emits [`ChatCompletionResponseChunk::ToolCall`] items (adapters MUST synthesize)
/// 4. zero or more [`ChatCompletionResponseChunk::ToolCall`] (finalized calls, stable order)
/// 5. exactly one [`ChatCompletionResponseChunk::Done`] with a [`ChatCompletionFinishReason`]
///
/// For text-only rounds, [`ChatCompletionResponseChunk::TextDone`] MAY appear immediately before [`ChatCompletionResponseChunk::Done`]
/// or be omitted (adapter-defined).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(tag = "type", content = "content", rename_all = "snake_case")
)]
pub enum ChatCompletionResponseChunk {
    TextBlock(String),
    Think(String),
    ThinkDone,
    TextDone,
    ToolCall(ToolCallRequest),
    Done { reason: ChatCompletionFinishReason },
}

/// Streaming completion provider (vendor implementations).
#[async_trait]
pub trait ChatCompletion: Send + Sync {
    async fn completion(
        &self,
        messages: &[ChatCompletionRequestMessage],
        tools: &[ToolManifest],
        stream: bool,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<ChatCompletionResponseChunk, MorayError>> + Send>>,
        MorayError,
    >;
}
