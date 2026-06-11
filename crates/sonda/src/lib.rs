#![forbid(unsafe_code)]
//! **Sonda** — TOML-backed multi-agent application framework for Moray desktop.

mod agent_runner;
mod completion_factory;
mod context;
mod error;
mod toolbox_factory;
mod session_catalog;
mod session_workspace;
mod session_factory;
mod settings_store;
mod skill_center;
mod snapshot;
mod sonda;
mod tool_catalog;
mod transcripts;

pub use error::{
    BadEnvironmentVariable, FileIoError, InvalidArguments, InvalidContent, MissingReference,
    Result, SondaError, SondaSessionError,
};
pub use agent_runner::{SondaAgentRunner, RUN_SUB_AGENT_TOOL_NAME};
pub use completion_factory::{SondaCompletionFactory, SondaCompletionRegistration};
pub use context::ContextBuilder;
pub use toolbox_factory::{
    SondaToolCatalogEntry, SondaToolRegistration, SondaToolboxFactory,
};
pub use session_catalog::{
    SessionAgentsConfig, SessionCatalogEntry, SessionCatalogError, SessionSubAgentEntry,
    SondaSessionCatalog, SubAgentContextMode,
};
pub use session_workspace::{
    ensure_session_dirs, SessionWorkspaceEntry, SessionWorkspacePath, SessionWorkspaceTree,
    SondaSessionWorkspace, SESSION_OUTPUT_DIR, SESSION_RESOURCES_DIR,
};
pub use session_factory::SondaSessionFactory;
pub use settings_store::{
    SondaSettingsAgentEntry, SondaSettingsCompletionEntry, SondaSettingsFile, SondaSettingsStore,
    SondaSettingsStoreError,
};
pub use skill_center::{
    SkillCatalogEntry, SkillCenter, SkillDetailEntry, SkillDirKind, SkillDirSource,
    SkillFilterKind, UnregisterSkillError,
};
pub use snapshot::{SondaSnapshot, SondaStateEvent};
pub use sonda::{
    CreateChannelResult, InstallSkillError, Sonda, SondaBuilder, UninstallSkillError,
    UninstallSkillResult,
};
pub use tool_catalog::{SondaToolCatalog, SondaToolCatalogError};
pub use transcripts::{
    replay_records, SondaSessionEventRecord, SondaSessionTranscripts, SondaSessionTranscriptsError,
};
