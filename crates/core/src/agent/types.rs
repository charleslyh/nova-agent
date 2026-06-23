use crate::completion::ChatCompletionResponseChunk;
use crate::toolbox::ToolCallEvent;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AgentFinishKind {
    Succeeded,
    Canceled,
    Refused {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        reason: Option<String>,
    },
    Failed {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "type", rename_all = "snake_case"))]
pub enum AgentResponseEvent {
    Started,
    CompletionResponse { chunk: ChatCompletionResponseChunk },
    ToolCall { event: ToolCallEvent },
    Finished { kind: AgentFinishKind },
}
