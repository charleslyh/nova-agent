//! Session orchestration: runtime and agent runner wiring.

mod runner;
mod error;
mod factory;
mod live;
mod runtime;

pub use runner::AgentRunner;
pub use error::{Result, SessionError};
pub use factory::SessionFactory;
pub use live::LiveSessions;
pub use runtime::{
    SessionEvent, SessionEventKind, SessionEventSink, SessionRuntime, TurnInput, TurnResource,
};
