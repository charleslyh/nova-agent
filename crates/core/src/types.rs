#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MorayError {
    #[error("{0}")]
    Message(String),

    /// Another [`Agent::run`](crate::Agent::run) stream is still active.
    #[error("agent is busy: another run stream is active")]
    Busy,
}

/// Tool metadata for listing and for chat-completion `tools` payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ToolManifest {
    pub name: String,
    pub description: String,
    /// JSON Schema object as a string (aligned with MCP `parameters`).
    pub parameters: String,
}

/// Tool call emitted by the model (finished stream) and tracked in assistant context.
///
/// `arguments` is the semantic JSON value (typically an object) after completion adapters parse
/// provider-specific wire forms. Produced when finalizing a model round, and by
/// [`Toolbox::call_tool`] for [`ToolCallEventKind::Requested`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ToolCallRequest {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

/// Shared internal type for completed tool-call outputs.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ToolCallStatus {
    Success,
    Error,
    Canceled,
}

/// Unified tool execution result payload used across toolbox events and agent internals.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ToolCallResult {
    pub call_id: String,
    pub content: String,
    pub status: ToolCallStatus,
}
