//! Session orchestration: runtime and agent runner wiring.

mod error;
mod factory;
mod live;
mod runtime;
mod runner;

pub use error::{Result, SessionError};
pub use factory::SessionFactory;
pub use live::LiveSessions;
pub use moray_core::{AgentRole, MultiAgentResponseEvent};
pub use runner::AgentRunner;
pub use runtime::{
    SessionAgentResponse, SessionEvent, SessionEventKind, SessionEventSink,
    SessionRuntime, TurnInput, TurnResource,
};
