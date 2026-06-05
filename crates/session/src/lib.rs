//! Session orchestration: runtime and harness wiring.

mod error;
mod factory;
mod harness;
mod live;
mod runtime;

pub use error::{Result, SessionError};
pub use factory::SessionFactory;
pub use harness::Harness;
pub use live::LiveSessions;
pub use runtime::{
    SessionEvent, SessionEventKind, SessionEventSink, SessionRuntime, TurnInput, TurnResource,
};
