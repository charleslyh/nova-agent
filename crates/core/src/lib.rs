//! Agent core: ReAct loop via [`AgentRequestBuilder`] over [`ChatCompletion`] and [`Toolbox`].
#![forbid(unsafe_code)]

mod agent;
mod completion;
mod context;
mod toolbox;
mod types;

pub use agent::{
    AgentEventSink, AgentFinishKind, AgentHarnessFactory, AgentRequestBuilder,
    AgentResponseEvent, AgentRole, AgentRunner, ChannelMultiAgentEventSink, MultiAgentEventSink,
    MultiAgentResponseEvent, MultiAgentsRequestBuilder, SubAgentContextMode, SubAgentSpec,
    RUN_SUB_AGENT_TOOL_NAME,
};
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
    MorayError, ToolCallRequest, ToolCallResult, ToolCallStatus, ToolManifest,
};
