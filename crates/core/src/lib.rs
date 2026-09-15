//! Agent core: ReAct loop via [`AgentRequestBuilder`] over [`ChatCompletion`] and [`Toolbox`].
#![forbid(unsafe_code)]

mod agent;
mod completion;
mod context;
mod tool_context;
mod toolbox;
mod types;

pub use agent::{
    AgentEventSink, AgentFinishKind, AgentHarnessFactory, AgentRequestBuilder, AgentResponseEvent,
    AgentResultFormatter, AgentRole, ChannelMultiAgentEventSink, MultiAgentEventSink,
    MultiAgentResponseEvent, MultiAgentsRequestBuilder, SubAgentContextMode, SubAgentRunData,
    SubAgentSpec, ToolCallRecord, RUN_SUB_AGENT_TOOL_NAME,
};
pub use completion::{
    ChatCompletion, ChatCompletionFinishReason, ChatCompletionRequestMessage,
    ChatCompletionResponseChunk, ChatCompletionUsage,
};
pub use context::ContextEngine;
pub use tool_context::ToolContext;
pub use toolbox::{
    Tool, ToolCallEvent, ToolCallEventKind, ToolCallEventSink, ToolCallGroupId,
    ToolCallInterceptor, ToolCallResponder, Toolbox, ToolboxBuilder, ToolboxError, TypedTool,
    TOOL_CALL_CANCELED, TOOL_CALL_DENIED_BY_USER,
};
pub use types::{NovaError, ToolCallRequest, ToolCallResult, ToolCallStatus, ToolManifest};
