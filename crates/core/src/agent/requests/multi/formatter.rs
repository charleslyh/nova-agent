use serde_json::Value;

use crate::types::ToolCallStatus;

/// Aggregated record of a single tool call's request and result.
///
/// Collected during the sub-agent's ReAct loop from streamed [`ToolCallEvent`] lifecycle events.
/// First phase captures: call_id, name, arguments, accumulated content, and final status.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ToolCallRecord {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
    /// Accumulated tool output content (from `Payload` events).
    pub content: String,
    /// Final status of the tool call. `None` if the call was never finished (e.g. agent canceled mid-call).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub status: Option<ToolCallStatus>,
}

/// Intermediate data collected during a sub-agent run, passed to [`AgentResultFormatter`].
///
/// This structure is designed for forward extension — new fields (e.g. user messages,
/// AI responses) can be added in future phases without breaking the formatter interface.
#[derive(Clone, Debug, Default)]
pub struct SubAgentRunData {
    /// All completed (or partially completed) tool calls observed during the run.
    pub tool_calls: Vec<ToolCallRecord>,
}

/// Trait for formatting a sub-agent's final result from its intermediate run data.
///
/// Implementations can produce structured output (e.g. JSON) from the raw tool-call records
/// rather than relying on the sub-agent's last text response.
pub trait AgentResultFormatter: Send + Sync {
    /// Format the sub-agent run data into the final result string returned to the leader.
    ///
    /// Returns `Ok(formatted)` on success, or `Err(reason)` on formatting failure.
    fn format(&self, data: &SubAgentRunData) -> std::result::Result<String, String>;
}
