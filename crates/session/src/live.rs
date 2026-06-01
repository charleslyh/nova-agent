//! In-process [`SessionRuntime`] registry: recall, evict, and turn control.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::{Result, SessionFactory, SessionRuntime, TurnInput};

/// Live session runtimes keyed by `session_id`.
#[derive(Clone)]
pub struct LiveSessions {
    sessions: Arc<RwLock<HashMap<String, Arc<SessionRuntime>>>>,
    factory: Arc<dyn SessionFactory>,
}

impl LiveSessions {
    pub fn new(factory: Arc<dyn SessionFactory>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            factory,
        }
    }

    pub fn evict(&self, session_id: &str) {
        let mut guard = self
            .sessions
            .write()
            .expect("live sessions lock poisoned");
        if let Some(session) = guard.remove(session_id) {
            let _ = session.cancel();
        }
    }

    pub async fn submit(
        &self,
        session_id: &str,
        input: TurnInput,
    ) -> Result<()> {
        let session = self.recall(session_id)?;
        session.submit(input).await?;
        Ok(())
    }

    pub fn cancel(&self, session_id: &str) -> Result<()> {
        let session = self.recall(session_id)?;
        session.cancel()?;
        Ok(())
    }

    pub async fn reset(&self, session_id: &str) -> Result<()> {
        let session = self.recall(session_id)?;
        session.reset().await?;
        Ok(())
    }

    fn recall(&self, session_id: &str) -> Result<Arc<SessionRuntime>> {
        if let Some(session) = self
            .sessions
            .read()
            .expect("live sessions lock poisoned")
            .get(session_id)
            .cloned()
        {
            return Ok(session);
        }

        let session = self.factory.create_session(session_id)?;

        self.sessions
            .write()
            .expect("live sessions lock poisoned")
            .insert(session_id.to_string(), session.clone());

        Ok(session)
    }
}
