//! Agent execution: ReAct loop, single- and multi-agent request builders.

mod react;
mod requests;
mod types;

pub use requests::multi::{
    AgentHarnessFactory, AgentResultFormatter, AgentRole, ChannelMultiAgentEventSink,
    MultiAgentEventSink, MultiAgentResponseEvent, MultiAgentsRequestBuilder, SubAgentContextMode,
    SubAgentRunData, SubAgentSpec, ToolCallRecord, RUN_SUB_AGENT_TOOL_NAME,
};
pub use requests::single::{AgentEventSink, AgentRequestBuilder};
pub use types::{AgentFinishKind, AgentResponseEvent};
