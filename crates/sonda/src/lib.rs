#![forbid(unsafe_code)]
//! **Sonda** — TOML-backed multi-agent application framework for Moray desktop.

mod error;
mod harness;
mod session_catalog;
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
pub use harness::{SondaSessionHarness, SondaToolCatalogEntry, SondaToolRegistration};
pub use session_catalog::{SessionCatalogEntry, SessionCatalogError, SondaSessionCatalog};
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
