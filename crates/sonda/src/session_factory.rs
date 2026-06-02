//! [`SondaSessionFactory`] — builds live [`SessionRuntime`] values for [`LiveSessions`].

use std::sync::Arc;

use moray_extensions::context::CompositeContextEngineBuilder;
use moray_extensions::preambles::{SkillsSection, TemplatedPreamblerBuilder};
use moray_core::ContextEngine;
use moray_session::{Harness, SessionFactory, SessionRuntime};

use crate::{
    replay_records, SkillCenter, SkillFilterKind, SondaSessionCatalog, SondaSessionError,
    SondaSessionHarness, SondaSessionTranscripts, SondaSettingsStore,
};

/// Session-scoped dependencies used when activating a live runtime.
#[derive(Clone)]
pub struct SondaSessionFactory {
    settings_store: Arc<SondaSettingsStore>,
    skill_center: SkillCenter,
    session_catalog: Arc<SondaSessionCatalog>,
    session_transcripts: Arc<SondaSessionTranscripts>,
    harness: Arc<SondaSessionHarness>,
}

impl SondaSessionFactory {
    pub fn new(
        settings_store: Arc<SondaSettingsStore>,
        skill_center: SkillCenter,
        session_catalog: Arc<SondaSessionCatalog>,
        session_transcripts: Arc<SondaSessionTranscripts>,
        harness: Arc<SondaSessionHarness>,
    ) -> Self {
        Self {
            settings_store,
            skill_center,
            session_catalog,
            session_transcripts,
            harness,
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

        let session_id = session_id.to_string();
        let settings = self.settings_store.clone();
        let session_catalog = self.session_catalog.clone();
        let skills = self.skill_center.skills(SkillFilterKind::All);
        let preamble_template = settings.preamble_template();
        let preambler = TemplatedPreamblerBuilder::default()
            .template(preamble_template)
            .with_fn("character", move || {
                let agent_id = match session_catalog.get_session_agent_id(session_id.as_str()) {
                    Ok(id) => id,
                    Err(_) => return String::new(),
                };
                settings
                    .agent_character(&agent_id)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
            })
            .section(SkillsSection::new(skills))
            .build();

        Ok(Arc::new(
            CompositeContextEngineBuilder::new()
                .messages(messages)
                .node(Arc::new(preambler))
                .build(),
        ))
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

        let context_engine = self.create_context_engine(session_id)?;
        let harness: Arc<dyn Harness> = self.harness.clone();

        Ok(Arc::new(SessionRuntime::new(
            session_id,
            self.session_transcripts.clone(),
            context_engine,
            harness,
            true,
        )))
    }
}
