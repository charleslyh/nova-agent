//! Agent core: ReAct loop via [`Agent::run`](crate::Agent::run) and [`AgentRequestBuilder`](crate::AgentRequestBuilder) over [`ChatCompletion`] and [`Toolbox`].
#![forbid(unsafe_code)]

mod agent;
mod completion;
mod context;
mod toolbox;
mod types;

pub use agent::{AgentFinishKind, AgentRequestBuilder, AgentResponseEvent};
pub use completion::{
    ChatCompletion, ChatCompletionFinishReason, ChatCompletionRequestMessage,
    ChatCompletionResponseChunk,
};
pub use context::ContextEngine;
pub use toolbox::{
    Tool, ToolCallAuthError, ToolCallAuthorizer, ToolCallEvent, ToolCallEventKind,
    ToolCallEventSink, ToolCallGroupId, ToolCallResponder, Toolbox,
    ToolboxBuilder, ToolboxError, TypedTool, TOOL_CALL_CANCELED, TOOL_CALL_DENIED_BY_USER,
};
pub use types::{
    parse_tool_call_args, MorayError, ToolCallRequest, ToolCallResult, ToolCallStatus,
    ToolManifest,
};
