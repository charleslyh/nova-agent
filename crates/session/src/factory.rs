use std::sync::Arc;

use crate::{error::Result, SessionRuntime};

/// Assembles in-process [`SessionRuntime`] values (harness, context, event writer).
pub trait SessionFactory: Send + Sync {
    fn create_session(&self, session_id: &str) -> Result<Arc<SessionRuntime>>;
}
