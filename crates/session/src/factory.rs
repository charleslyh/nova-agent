use std::sync::Arc;

use crate::{error::Result, SessionRuntime};

pub trait SessionFactory: Send + Sync {
    fn create_session(&self, session_id: &str) -> Result<Arc<SessionRuntime>>;
}
