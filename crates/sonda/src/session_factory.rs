//! [`SondaSessionFactory`] — builds live [`SessionRuntime`] values for [`LiveSessions`].

use std::sync::Arc;

use moray_core::ContextEngine;
use moray_session::{SessionFactory, SessionRuntime};

use crate::{
    context::ContextBuilder,
    replay_records, SondaAgentRunner, SondaSessionCatalog, SondaSessionError,
    SondaSessionTranscripts,
};

/// Session-scoped dependencies used when activating a live runtime.
#[derive(Clone)]
pub struct SondaSessionFactory {
    session_catalog: Arc<SondaSessionCatalog>,
    session_transcripts: Arc<SondaSessionTranscripts>,
    agent_runner: Arc<SondaAgentRunner>,
    context_builder: ContextBuilder,
}

impl SondaSessionFactory {
    pub fn new(
        session_catalog: Arc<SondaSessionCatalog>,
        session_transcripts: Arc<SondaSessionTranscripts>,
        agent_runner: Arc<SondaAgentRunner>,
        context_builder: ContextBuilder,
    ) -> Self {
        Self {
            session_catalog,
            session_transcripts,
            agent_runner,
            context_builder,
        }
    }

    fn create_context_engine(
        &self,
        session_id: &str,
    ) -> std::result::Result<Arc<dyn ContextEngine>, SondaSessionError> {
        let (records, _) = self
            .session_transcripts
            .load(session_id)
            .map_err(|e| SondaSessionError::Moray(e.into()))?;

        let messages = replay_records(&records).messages;
        let agent_id = self
            .session_catalog
            .get_session_agent_id(session_id)
            .map_err(|e| SondaSessionError::Moray(crate::SondaError::from(e).into()))?;

        (self.context_builder)(agent_id.as_str(), messages)
            .map_err(|e| SondaSessionError::Moray(e.into()))
    }
}

impl SessionFactory for SondaSessionFactory {
    fn create_session(
        &self,
        session_id: &str,
    ) -> std::result::Result<Arc<SessionRuntime>, SondaSessionError> {
        if !self.session_catalog.has_session(session_id) {
            return Err(SondaSessionError::UnknownSession);
        }

        // 在创建 session 时，就应该立即创建 leader agent 的 context engine，并可以在所有后续 turn 中复用。
        // 从而可以在整个 session 生命周期中跟踪完整的上下文状态。
        let context_engine = self.create_context_engine(session_id)?;

        Ok(Arc::new(SessionRuntime::new(
            session_id,
            self.session_transcripts.clone(),
            context_engine,
            self.agent_runner.clone(),
        )))
    }
}
