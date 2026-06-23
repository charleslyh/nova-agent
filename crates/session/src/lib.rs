//! Session orchestration: runtime and agent runner wiring.

mod error;
mod factory;
mod live;
mod runtime;

pub use moray_core::AgentRunner;
pub use error::{Result, SessionError};
pub use factory::SessionFactory;
pub use live::LiveSessions;
pub use moray_core::{AgentRole, MultiAgentResponseEvent};
pub use runtime::{
    SessionAgentResponse, SessionEvent, SessionEventKind, SessionEventSink,
    SessionRuntime, TurnInput, TurnResource,
};
